use super::*;

#[tokio::test]
async fn create_library_rejects_root_used_by_other_facet_library() {
    let (app, user) = bootstrap();
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let series_library = app
        .services
        .catalog
        .libraries
        .get_by_id(&series_library_id)
        .await
        .expect("series library should load")
        .expect("series library should exist");

    app.services
        .catalog
        .libraries
        .update(
            &series_library_id,
            series_library.name.clone(),
            series_library.slug.clone(),
            vec![LibraryRootDraft {
                id: None,
                path: "/Volumes/Media/TV".to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("series library roots should update");

    let error = app
        .create_library(
            &user,
            MediaFacet::Anime,
            "Anime2".to_string(),
            vec![LibraryRootDraft {
                id: None,
                path: "/Volumes/Media/TV".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect_err("duplicate cross-facet root should be rejected");

    match error {
        AppError::Validation(message) => {
            assert!(message.contains("/Volumes/Media/TV"));
            assert!(message.contains(&series_library.name));
        }
        other => panic!("expected validation error, got {other:?}"),
    }
}

#[tokio::test]
async fn update_library_rejects_root_used_by_other_facet_library() {
    let (app, user) = bootstrap();
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let anime_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    let series_library = app
        .services
        .catalog
        .libraries
        .get_by_id(&series_library_id)
        .await
        .expect("series library should load")
        .expect("series library should exist");
    let anime_library = app
        .services
        .catalog
        .libraries
        .get_by_id(&anime_library_id)
        .await
        .expect("anime library should load")
        .expect("anime library should exist");

    app.services
        .catalog
        .libraries
        .update(
            &series_library_id,
            series_library.name.clone(),
            series_library.slug.clone(),
            vec![LibraryRootDraft {
                id: None,
                path: "/Volumes/Media/TV".to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("series library roots should update");

    let error = app
        .update_library(
            &user,
            &anime_library_id,
            Some(anime_library.name.clone()),
            Some(vec![LibraryRootDraft {
                id: None,
                path: "/Volumes/Media/TV".to_string(),
                is_default: true,
            }]),
            None,
        )
        .await
        .expect_err("duplicate cross-facet root should be rejected");

    match error {
        AppError::Validation(message) => {
            assert!(message.contains("/Volumes/Media/TV"));
            assert!(message.contains(&series_library.name));
        }
        other => panic!("expected validation error, got {other:?}"),
    }
}

#[tokio::test]
async fn delete_library_rejects_default_library() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let error = app
        .delete_library(&user, &movie_library_id)
        .await
        .expect_err("default library delete should be rejected");

    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("default libraries cannot be deleted")),
        "unexpected delete error: {error:?}"
    );
}

#[tokio::test]
async fn delete_library_purges_library_state_for_non_default_library() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, mut user) = bootstrap_with_scan_unmatched_tracking(
        settings.clone(),
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );

    let library = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Kids".to_string(),
            vec![LibraryRootDraft {
                id: None,
                path: "/Volumes/Media/Kids".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("library should be created");

    app.services
        .catalog
        .libraries
        .set_grants_for_user(
            &user.id,
            vec![scryer_domain::LibraryGrant {
                user_id: user.id.clone(),
                library_id: library.id.clone(),
                permissions: scryer_domain::LibraryPermissionMask::from_permissions([
                    scryer_domain::LibraryPermission::View,
                    scryer_domain::LibraryPermission::ManageTitles,
                    scryer_domain::LibraryPermission::ManageLibrary,
                ]),
            }],
        )
        .await
        .expect("library grants should be stored");
    user.authorization.loaded = false;

    settings
        .set_scoped_value("system", "quality.profile", &library.id, "\"kids\"")
        .await;

    let created = app
        .create_title_without_hydration_in_library(
            &user,
            NewTitle {
                name: "Delete Me".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
            library.id.clone(),
        )
        .await
        .expect("title should be created");

    let mut pending_item = build_test_unmatched_item(
        "library-delete-unmatched",
        MediaFacet::Movie,
        "/Volumes/Media/Kids",
        "/Volumes/Media/Kids/Delete.Me.2026.mkv",
        "Delete Me",
        "Delete Me",
        Some(2026),
    );
    pending_item.library_id = library.id.clone();
    unmatched_items
        .upsert_library_scan_unmatched_item(&pending_item)
        .await
        .expect("pending import should be stored");

    let deleted = app
        .delete_library(&user, &library.id)
        .await
        .expect("library delete should succeed");
    assert!(deleted);

    assert!(
        app.services
            .catalog
            .libraries
            .get_by_id(&library.id)
            .await
            .expect("library lookup should succeed")
            .is_none()
    );
    assert!(
        app.services
            .catalog
            .titles
            .get_by_id(&created.title.id)
            .await
            .expect("title lookup should succeed")
            .is_none()
    );
    assert!(
        settings
            .get_scoped_value("system", "quality.profile", &library.id)
            .await
            .is_none()
    );
    assert!(
        unmatched_items
            .items()
            .await
            .iter()
            .all(|item| item.library_id != library.id)
    );
    assert!(
        app.services
            .catalog
            .libraries
            .permission_masks_for_user(&user.id)
            .await
            .expect("grant lookup should succeed")
            .iter()
            .all(|grant| grant.library_id != library.id)
    );
}

#[tokio::test]
async fn delete_library_purges_history_before_deleting_title_rows() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let titles = Arc::new(MockTitleRepo::default());
    let operation_log = Arc::new(Mutex::new(Vec::new()));
    titles.set_delete_operation_log(operation_log.clone()).await;

    let domain_events = Arc::new(MockDomainEventRepo::default());
    domain_events
        .set_delete_operation_log(operation_log.clone())
        .await;

    let (app, mut user) = bootstrap_with_library_delete_repositories(
        titles,
        settings,
        unmatched_items,
        domain_events,
        Arc::new(TrackingHousekeepingRepo::with_operation_log(
            operation_log.clone(),
        )),
        Arc::new(TrackingPendingReleaseRepo::default()),
    );

    let library = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Kids".to_string(),
            vec![LibraryRootDraft {
                id: None,
                path: "/Volumes/Media/Kids".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("library should be created");

    app.services
        .catalog
        .libraries
        .set_grants_for_user(
            &user.id,
            vec![scryer_domain::LibraryGrant {
                user_id: user.id.clone(),
                library_id: library.id.clone(),
                permissions: scryer_domain::LibraryPermissionMask::from_permissions([
                    scryer_domain::LibraryPermission::View,
                    scryer_domain::LibraryPermission::ManageTitles,
                    scryer_domain::LibraryPermission::ManageLibrary,
                ]),
            }],
        )
        .await
        .expect("library grants should be stored");
    user.authorization.loaded = false;

    app.create_title_without_hydration_in_library(
        &user,
        NewTitle {
            name: "Delete Me".into(),
            facet: MediaFacet::Movie,
            monitored: false,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,
            ..Default::default()
        },
        library.id.clone(),
    )
    .await
    .expect("title should be created");

    let deleted = app
        .delete_library(&user, &library.id)
        .await
        .expect("library delete should succeed");
    assert!(deleted);

    let operations = operation_log.lock().await.clone();
    let delete_title_index = operations
        .iter()
        .position(|entry| entry.starts_with("delete_title:"))
        .expect("title delete should be recorded");

    assert!(operations[..delete_title_index].contains(&"delete_domain_events".to_string()));
    assert!(operations[..delete_title_index].contains(&"delete_history_events".to_string()));
    assert!(
        operations[..delete_title_index].contains(&"delete_download_import_artifacts".to_string())
    );
    assert!(operations[..delete_title_index].contains(&"delete_release_attempts".to_string()));
}

#[tokio::test]
async fn delete_library_returns_error_when_title_dependency_cleanup_fails() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let titles = Arc::new(MockTitleRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    pending_releases
        .fail_delete_for_title("pending release cleanup failed")
        .await;

    let (app, mut user) = bootstrap_with_library_delete_repositories(
        titles,
        settings.clone(),
        unmatched_items,
        Arc::new(MockDomainEventRepo::default()),
        Arc::new(TrackingHousekeepingRepo::default()),
        pending_releases,
    );

    let library = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Kids".to_string(),
            vec![LibraryRootDraft {
                id: None,
                path: "/Volumes/Media/Kids".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("library should be created");

    app.services
        .catalog
        .libraries
        .set_grants_for_user(
            &user.id,
            vec![scryer_domain::LibraryGrant {
                user_id: user.id.clone(),
                library_id: library.id.clone(),
                permissions: scryer_domain::LibraryPermissionMask::from_permissions([
                    scryer_domain::LibraryPermission::View,
                    scryer_domain::LibraryPermission::ManageTitles,
                    scryer_domain::LibraryPermission::ManageLibrary,
                ]),
            }],
        )
        .await
        .expect("library grants should be stored");
    user.authorization.loaded = false;

    settings
        .set_scoped_value("system", "quality.profile", &library.id, "\"kids\"")
        .await;

    let created = app
        .create_title_without_hydration_in_library(
            &user,
            NewTitle {
                name: "Delete Me".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
            library.id.clone(),
        )
        .await
        .expect("title should be created");

    let error = app
        .delete_library(&user, &library.id)
        .await
        .expect_err("library delete should fail");

    assert!(
        matches!(error, AppError::Repository(ref message) if message.contains("pending release cleanup failed")),
        "unexpected delete error: {error:?}"
    );
    assert!(
        app.services
            .catalog
            .libraries
            .get_by_id(&library.id)
            .await
            .expect("library lookup should succeed")
            .is_some()
    );
    assert!(
        app.services
            .catalog
            .titles
            .get_by_id(&created.title.id)
            .await
            .expect("title lookup should succeed")
            .is_some()
    );
    assert!(
        settings
            .get_scoped_value("system", "quality.profile", &library.id)
            .await
            .is_some()
    );
}

#[tokio::test]
async fn update_default_library_preserves_default_slug() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let updated = app
        .update_library(
            &user,
            &movie_library_id,
            Some("Main Movies".to_string()),
            None,
            None,
        )
        .await
        .expect("default library rename should succeed");

    assert_eq!(updated.name, "Main Movies");
    assert_eq!(
        updated.slug,
        scryer_domain::default_library_slug_for_facet(&MediaFacet::Movie)
    );
}

#[tokio::test]
async fn update_non_default_library_rederives_slug_from_name() {
    let (app, user) = bootstrap();
    let created = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Kids Movies".to_string(),
            vec![LibraryRootDraft {
                id: None,
                path: "/Volumes/Media/Kids".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("custom library should be created");

    let updated = app
        .update_library(
            &user,
            &created.id,
            Some("Adult Movies".to_string()),
            None,
            None,
        )
        .await
        .expect("custom library rename should succeed");

    assert_eq!(updated.name, "Adult Movies");
    assert_eq!(updated.slug, "adult-movies");
}

#[tokio::test]
async fn library_sidecar_settings_resolve_facet_defaults_and_library_overrides() {
    let (app, user) = bootstrap();
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);

    app.update_media_settings(
        &user,
        MediaFacet::Series,
        UpdateMediaSettings {
            nfo_write_on_import: Some(true),
            plexmatch_write_on_import: Some(true),
            ..empty_update_media_settings()
        },
    )
    .await
    .expect("series media settings should update");

    let baseline = app
        .get_library_settings(&user, &series_library_id)
        .await
        .expect("series library settings should load");
    assert_eq!(baseline.nfo_write_on_import_override, None);
    assert!(baseline.nfo_write_on_import);
    assert_eq!(baseline.plexmatch_write_on_import_override, None);
    assert_eq!(baseline.plexmatch_write_on_import, Some(true));

    app.update_library_settings(
        &user,
        &series_library_id,
        LibrarySettingsOverrideDraft {
            nfo_write_on_import: Some(false),
            plexmatch_write_on_import: Some(false),
            ..empty_library_settings_override()
        },
    )
    .await
    .expect("series library overrides should save");

    let overridden = app
        .get_library_settings(&user, &series_library_id)
        .await
        .expect("series library settings should reload");
    assert_eq!(overridden.nfo_write_on_import_override, Some(false));
    assert!(!overridden.nfo_write_on_import);
    assert_eq!(overridden.plexmatch_write_on_import_override, Some(false));
    assert_eq!(overridden.plexmatch_write_on_import, Some(false));
}

#[tokio::test]
async fn import_mode_settings_resolve_default_facet_override_and_library_override() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let default_media_settings = app
        .get_media_settings(&user, MediaFacet::Movie)
        .await
        .expect("movie media settings should load");
    assert_eq!(
        default_media_settings.import_mode,
        ImportMode::HardlinkOrCopy
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        UpdateMediaSettings {
            import_mode: Some(ImportMode::Move),
            ..empty_update_media_settings()
        },
    )
    .await
    .expect("movie import mode should update");

    let facet_override = app
        .get_library_settings(&user, &movie_library_id)
        .await
        .expect("movie library settings should load");
    assert_eq!(facet_override.import_mode_override, None);
    assert_eq!(facet_override.import_mode, ImportMode::Move);

    app.update_library_settings(
        &user,
        &movie_library_id,
        LibrarySettingsOverrideDraft {
            import_mode: Some(ImportMode::HardlinkOrCopy),
            ..empty_library_settings_override()
        },
    )
    .await
    .expect("movie library import mode override should save");

    let library_override = app
        .get_library_settings(&user, &movie_library_id)
        .await
        .expect("movie library settings should reload");
    assert_eq!(
        library_override.import_mode_override,
        Some(ImportMode::HardlinkOrCopy)
    );
    assert_eq!(library_override.import_mode, ImportMode::HardlinkOrCopy);

    app.update_library_settings(&user, &movie_library_id, empty_library_settings_override())
        .await
        .expect("movie library import mode override should clear");

    let inherited_again = app
        .get_library_settings(&user, &movie_library_id)
        .await
        .expect("movie library settings should reload after reset");
    assert_eq!(inherited_again.import_mode_override, None);
    assert_eq!(inherited_again.import_mode, ImportMode::Move);
}

#[tokio::test]
async fn import_mode_settings_reject_invalid_stored_value() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(SETTINGS_SCOPE_SYSTEM, IMPORT_MODE_KEY, "movie", "\"auto\"")
        .await;
    let (app, user) = bootstrap_with_settings_repo_and_profiles(
        settings,
        Arc::new(StoredQualityProfileRepo::default()),
        Arc::new(MockIndexerClient),
    );

    let error = app
        .get_media_settings(&user, MediaFacet::Movie)
        .await
        .expect_err("invalid import mode should be rejected");

    match error {
        AppError::Validation(message) => {
            assert!(message.contains("invalid import.mode setting value"));
        }
        other => panic!("expected validation error, got {other:?}"),
    }
}

#[tokio::test]
async fn resolve_quality_profile_uses_facet_settings_when_library_scope_only_coalesces_defaults() {
    let settings = Arc::new(CoalescingSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            "\"wizard-movie\"",
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            "movie",
            "\"wizard-movie\"",
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            "series",
            "\"wizard-series\"",
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            "anime",
            "\"wizard-anime\"",
        )
        .await;

    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![
            test_quality_profile("4k"),
            test_quality_profile("wizard-movie"),
            test_quality_profile("wizard-series"),
            test_quality_profile("wizard-anime"),
        ])
        .await;

    let (app, _) = bootstrap_with_settings_repo_and_profiles(
        settings,
        quality_profiles,
        Arc::new(MockIndexerClient),
    );

    for (facet, category_hint, expected_profile_id) in [
        (MediaFacet::Movie, "movie", "wizard-movie"),
        (MediaFacet::Series, "series", "wizard-series"),
        (MediaFacet::Anime, "anime", "wizard-anime"),
    ] {
        let library_id = scryer_domain::default_library_id_for_facet(&facet);
        let resolved = app
            .resolve_quality_profile(crate::app_usecase_discovery::QualityProfileLookup {
                title_tags: &[],
                library_id: Some(library_id.as_str()),
                imdb_id: None,
                tvdb_id: None,
                category_hint: Some(category_hint),
            })
            .await
            .expect("quality profile should resolve");

        assert_eq!(resolved.id, expected_profile_id);
    }
}

#[tokio::test]
async fn library_settings_inherit_facet_quality_and_persona_when_library_scope_only_coalesces_defaults()
 {
    let settings = Arc::new(CoalescingSettingsRepo::default());
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);

    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            "\"wizard-movie\"",
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            "series",
            "\"wizard-series\"",
        )
        .await;
    settings
        .set_value(SETTINGS_SCOPE_SYSTEM, SCORING_PERSONA_KEY, "\"Compatible\"")
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            SCORING_PERSONA_KEY,
            "series",
            "\"Audiophile\"",
        )
        .await;

    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![
            test_quality_profile("4k"),
            test_quality_profile("wizard-movie"),
            test_quality_profile("wizard-series"),
        ])
        .await;

    let (app, user) = bootstrap_with_settings_repo_and_profiles(
        settings,
        quality_profiles,
        Arc::new(MockIndexerClient),
    );

    let library_settings = app
        .get_library_settings(&user, &series_library_id)
        .await
        .expect("library settings should load");

    assert_eq!(library_settings.quality_profile_id_override, None);
    assert_eq!(library_settings.quality_profile_id, "wizard-series");
    assert_eq!(library_settings.scoring_persona_override, None);
    assert_eq!(library_settings.scoring_persona, ScoringPersona::Audiophile);
}

#[tokio::test]
async fn library_settings_inherit_facet_routing_when_library_scope_only_coalesces_defaults() {
    let settings = Arc::new(CoalescingSettingsRepo::default());
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);

    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "series",
            r#"{"weaver":{"enabled":true,"category":"tv"}}"#,
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            INDEXER_ROUTING_SETTINGS_KEY,
            "series",
            r#"{"nzbgeek":{"enabled":true,"categories":["5000"],"priority":7}}"#,
        )
        .await;

    let (app, user) = bootstrap_with_settings_repo_and_profiles(
        settings,
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
    );

    let download_client_routing = app
        .get_download_client_routing(&user, "series")
        .await
        .expect("download client routing should load");
    assert_eq!(download_client_routing.len(), 1);
    assert_eq!(download_client_routing[0].client_id, "weaver");

    let indexer_routing = app
        .get_indexer_routing(&user, "series")
        .await
        .expect("indexer routing should load");
    assert_eq!(indexer_routing.len(), 1);
    assert_eq!(indexer_routing[0].indexer_id, "nzbgeek");

    let library_settings = app
        .get_library_settings(&user, &series_library_id)
        .await
        .expect("library settings should load");

    assert_eq!(library_settings.download_client_routing_override, None);
    assert_eq!(library_settings.indexer_routing_override, None);
}

#[tokio::test]
async fn movie_library_rejects_plexmatch_override() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let error = app
        .update_library_settings(
            &user,
            &movie_library_id,
            LibrarySettingsOverrideDraft {
                plexmatch_write_on_import: Some(true),
                ..empty_library_settings_override()
            },
        )
        .await
        .expect_err("movie library should reject plexmatch override");

    match error {
        AppError::Validation(message) => {
            assert!(message.contains("plexmatch_write_on_import"));
        }
        other => panic!("expected validation error, got {other:?}"),
    }
}
