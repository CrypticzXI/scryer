use super::*;

#[tokio::test]
async fn update_title_metadata_changes_name_and_tags() {
    let (app, user) = bootstrap();
    let created = app
        .add_title(
            &user,
            NewTitle {
                name: "Original".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec!["SciFi".into()],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let updated = app
        .update_title_metadata(
            &user,
            &created.id,
            Some("Updated Name".into()),
            None,
            Some(vec!["Action".into(), "Drama".into(), "Action".into()]),
        )
        .await
        .expect("update title metadata");

    assert_eq!(updated.name, "Updated Name");
    assert_eq!(
        updated.tags,
        vec!["action".to_string(), "drama".to_string()]
    );
    let events = title_updated_events(&app, &created.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_user_id.as_deref(), Some(user.id.as_str()));
    assert!(matches!(
        &events[0].payload,
        DomainEventPayload::TitleUpdated(_)
    ));
}

#[tokio::test]
async fn set_primary_movie_file_promotes_selected_and_demotes_same_folder_files() {
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        media_files,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Primary Switch".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create movie title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, "/movies/Primary Switch (2026)")
        .await
        .expect("set folder path");

    let old_primary_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/movies/Primary Switch (2026)/Primary Switch 1080p.mkv".into(),
            size_bytes: 1_000,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert old primary");
    let new_primary_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/movies/Primary Switch (2026)/Primary Switch 2160p.mkv".into(),
            size_bytes: 2_000,
            role: MediaFileRole::Additional,
            ..Default::default()
        })
        .await
        .expect("insert additional file");
    let out_of_folder_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/movies/Primary Switch Copy/Primary Switch 720p.mkv".into(),
            size_bytes: 500,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert polluted out-of-folder file");

    app.set_primary_movie_file(&user, &title.id, &new_primary_id)
        .await
        .expect("promote primary movie file");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list files");
    let role_for = |file_id: &str| {
        files
            .iter()
            .find(|file| file.id == file_id)
            .map(|file| file.role)
            .expect("file role")
    };
    assert_eq!(role_for(&new_primary_id), MediaFileRole::Primary);
    assert_eq!(role_for(&old_primary_id), MediaFileRole::Additional);
    assert_eq!(role_for(&out_of_folder_id), MediaFileRole::Primary);

    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_user_id.as_deref(), Some(user.id.as_str()));
}

#[tokio::test]
async fn set_primary_movie_file_scopes_series_movie_promotion_to_linked_files() {
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        media_files,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Series Movie Primary Switch".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            &title.id,
            "Series Movie Primary Switch: The Movie",
            Some(2026),
            None,
            None,
        ))
        .await
        .expect("create series movie link");

    let old_primary_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/anime/Series Movie Primary Switch/Specials/primary.mkv".into(),
            size_bytes: 1_000,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert old primary");
    app.services
        .library
        .media_files
        .link_file_to_series_movie(&old_primary_id, &link.id)
        .await
        .expect("link old primary");
    let new_primary_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/anime/Series Movie Primary Switch/Specials/additional.mkv".into(),
            size_bytes: 2_000,
            role: MediaFileRole::Additional,
            ..Default::default()
        })
        .await
        .expect("insert additional file");
    app.services
        .library
        .media_files
        .link_file_to_series_movie(&new_primary_id, &link.id)
        .await
        .expect("link additional file");
    let unrelated_primary_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/anime/Series Movie Primary Switch/Season 01/episode.mkv".into(),
            size_bytes: 500,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert unrelated primary");

    app.set_primary_movie_file(&user, &title.id, &new_primary_id)
        .await
        .expect("promote series movie primary file");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list files");
    let role_for = |file_id: &str| {
        files
            .iter()
            .find(|file| file.id == file_id)
            .map(|file| file.role)
            .expect("file role")
    };
    assert_eq!(role_for(&new_primary_id), MediaFileRole::Primary);
    assert_eq!(role_for(&old_primary_id), MediaFileRole::Additional);
    assert_eq!(role_for(&unrelated_primary_id), MediaFileRole::Primary);
}

#[tokio::test]
async fn set_title_monitored_emits_title_updated_with_actor() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Monitor Fixture".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let updated = app
        .set_title_monitored(&user, &title.id, false)
        .await
        .expect("update monitored");

    assert!(!updated.monitored);
    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_user_id.as_deref(), Some(user.id.as_str()));
}

#[tokio::test]
async fn set_collection_monitored_emits_one_title_updated_with_actor() {
    let (app, user) = bootstrap();
    let (title, collection, _) =
        create_series_with_collection_and_episode(&app, &user, "Collection Monitor Fixture").await;

    let updated = app
        .set_collection_monitored(&user, &collection.id, false)
        .await
        .expect("update collection monitoring");

    assert!(!updated.monitored);
    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_user_id.as_deref(), Some(user.id.as_str()));
}

#[tokio::test]
async fn set_episode_monitored_emits_one_title_updated_with_actor() {
    let (app, user) = bootstrap();
    let (title, _, episode) =
        create_series_with_collection_and_episode(&app, &user, "Episode Monitor Fixture").await;

    let updated = app
        .set_episode_monitored(&user, &episode.id, false)
        .await
        .expect("update episode monitoring");

    assert!(!updated.monitored);
    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_user_id.as_deref(), Some(user.id.as_str()));
}

#[tokio::test]
async fn external_import_monitor_snapshot_emits_title_updated_without_actor() {
    let (app, user) = bootstrap();
    let snapshots = Arc::new(MockExternalImportMonitorSnapshotRepo::default());
    let app = app.with_test_overrides(|services| {
        services.with_external_import_monitor_snapshots(snapshots.clone())
    });

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Snapshot Monitor Fixture".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "4242".to_string(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");
    app.create_episode(
        &user,
        title.id.clone(),
        Some(collection.id),
        "standard".into(),
        Some("1".into()),
        Some("1".into()),
        Some("Pilot".into()),
        Some("Pilot".into()),
        None,
        Some(1_200),
        false,
        false,
    )
    .await
    .expect("create episode");

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("4242".to_string()),
            path: None,
            monitored: false,
            seasons: vec![],
            episodes: vec![],
        }],
    )
    .await;

    let applied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("apply monitor snapshot");

    assert!(applied);
    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 1);
    assert!(events.iter().all(|event| event.actor_user_id.is_none()));

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("4242".to_string()),
            path: None,
            monitored: false,
            seasons: vec![],
            episodes: vec![],
        }],
    )
    .await;

    let reapplied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("reapply monitor snapshot");

    assert!(reapplied);
    let replay_events = title_updated_events(&app, &title.id).await;
    assert_eq!(replay_events.len(), 1);
    assert!(
        replay_events
            .iter()
            .all(|event| event.actor_user_id.is_none())
    );
}

#[tokio::test]
async fn external_import_monitor_snapshot_applies_series_child_monitoring() {
    let (app, user) = bootstrap();
    let snapshots = Arc::new(MockExternalImportMonitorSnapshotRepo::default());
    let app = app.with_test_overrides(|services| {
        services.with_external_import_monitor_snapshots(snapshots.clone())
    });

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Snapshot Child Monitor Fixture".into(),
                facet: MediaFacet::Series,
                monitored: false,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "5252".to_string(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");
    app.create_episode(
        &user,
        title.id.clone(),
        Some(collection.id),
        "standard".into(),
        Some("1".into()),
        Some("1".into()),
        Some("Pilot".into()),
        Some("Pilot".into()),
        None,
        Some(1_200),
        false,
        false,
    )
    .await
    .expect("create episode");

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("5252".to_string()),
            path: None,
            monitored: true,
            seasons: vec![],
            episodes: vec![],
        }],
    )
    .await;

    let applied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("apply monitor snapshot");

    assert!(applied);
    let stored_title = app
        .services
        .catalog
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load title")
        .expect("title exists");
    let collections = app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
        .expect("list collections");
    let episodes = app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await
        .expect("list episodes");

    assert!(stored_title.monitored);
    assert!(collections.iter().any(|collection| collection.monitored));
    assert!(episodes.iter().any(|episode| episode.monitored));
}

#[tokio::test]
async fn external_import_monitor_snapshot_syncs_wanted_state_once_per_title() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
    );
    let snapshots = Arc::new(MockExternalImportMonitorSnapshotRepo::default());
    let app = app.with_test_overrides(|services| {
        services.with_external_import_monitor_snapshots(snapshots.clone())
    });

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Snapshot Sync Fixture".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "5150".to_string(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Series)
        .await;

    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("Pilot".into()),
            None,
            Some(1_200),
            false,
            false,
        )
        .await
        .expect("create episode");
    app.set_episode_monitored(&user, &episode.id, false)
        .await
        .expect("disable episode");

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("5150".to_string()),
            path: None,
            monitored: true,
            seasons: vec![ExternalImportMonitorSeasonEntry {
                season_number: 1,
                monitored: true,
            }],
            episodes: vec![ExternalImportMonitorEpisodeEntry {
                tvdb_id: None,
                season_number: 1,
                episode_number: 1,
                monitored: true,
            }],
        }],
    )
    .await;

    let upserts_before_apply = wanted_items.upsert_call_count();
    let applied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("apply monitor snapshot");

    assert!(applied);
    let upserts_after_apply = wanted_items.upsert_call_count();
    assert_eq!(upserts_after_apply - upserts_before_apply, 1);
}

#[tokio::test]
async fn external_import_monitor_snapshot_emits_title_updated_for_child_only_changes() {
    let (app, user) = bootstrap();
    let snapshots = Arc::new(MockExternalImportMonitorSnapshotRepo::default());
    let app = app.with_test_overrides(|services| {
        services.with_external_import_monitor_snapshots(snapshots.clone())
    });

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Snapshot Child Activity Fixture".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "6262".to_string(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("Pilot".into()),
            None,
            Some(1_200),
            false,
            false,
        )
        .await
        .expect("create episode");
    app.set_episode_monitored(&user, &episode.id, false)
        .await
        .expect("disable episode");

    let events_before_apply = title_updated_events(&app, &title.id).await.len();

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("6262".to_string()),
            path: None,
            monitored: true,
            seasons: vec![ExternalImportMonitorSeasonEntry {
                season_number: 1,
                monitored: true,
            }],
            episodes: vec![ExternalImportMonitorEpisodeEntry {
                tvdb_id: None,
                season_number: 1,
                episode_number: 1,
                monitored: true,
            }],
        }],
    )
    .await;

    let applied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("apply monitor snapshot");

    assert!(applied);
    let updated_episode = app
        .get_episode(&user, &episode.id)
        .await
        .expect("get episode")
        .expect("episode exists");
    assert!(updated_episode.monitored);

    let events_after_apply = title_updated_events(&app, &title.id).await;
    assert_eq!(events_after_apply.len(), events_before_apply + 1);
    assert!(
        events_after_apply
            .last()
            .expect("latest event")
            .actor_user_id
            .is_none()
    );
}

#[tokio::test]
async fn external_import_monitor_snapshot_enables_collection_for_monitored_episode_override() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
    );
    let snapshots = Arc::new(MockExternalImportMonitorSnapshotRepo::default());
    let app = app.with_test_overrides(|services| {
        services.with_external_import_monitor_snapshots(snapshots.clone())
    });

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Snapshot Episode Override Fixture".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "7373".to_string(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Series)
        .await;

    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id.clone()),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("Pilot".into()),
            None,
            Some(1_200),
            false,
            false,
        )
        .await
        .expect("create episode");
    app.set_collection_monitored(&user, &collection.id, false)
        .await
        .expect("disable collection");

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("7373".to_string()),
            path: None,
            monitored: false,
            seasons: vec![],
            episodes: vec![ExternalImportMonitorEpisodeEntry {
                tvdb_id: None,
                season_number: 1,
                episode_number: 1,
                monitored: true,
            }],
        }],
    )
    .await;

    let upserts_before_apply = wanted_items.upsert_call_count();
    let applied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("apply monitor snapshot");

    assert!(applied);
    let updated_collection = app
        .get_collection(&user, &collection.id)
        .await
        .expect("get collection")
        .expect("collection exists");
    let updated_episode = app
        .get_episode(&user, &episode.id)
        .await
        .expect("get episode")
        .expect("episode exists");
    assert!(updated_collection.monitored);
    assert!(updated_episode.monitored);

    let upserts_after_apply = wanted_items.upsert_call_count();
    assert_eq!(upserts_after_apply - upserts_before_apply, 1);
}
