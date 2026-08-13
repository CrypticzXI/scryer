use super::*;

#[tokio::test]
async fn list_cutoff_unmet_titles_normalizes_lowercase_cutoff_tier() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            r#""cutoff-lowercase""#,
        )
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile(
            "cutoff-lowercase",
            "720p",
        )])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Cutoff Case".into(),
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

    media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Cutoff Case.mkv".to_string(),
            size_bytes: 1_000,
            quality_label: Some("480p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");

    let items = app
        .list_cutoff_unmet_titles(&user, Some(MediaFacet::Movie), None)
        .await
        .expect("cutoff unmet query should succeed");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title_id, title.id);
    assert_eq!(items[0].episode_id, None);
    assert_eq!(items[0].current_tier, "480P");
    assert_eq!(items[0].target_tier, "720P");
}

#[tokio::test]
async fn list_cutoff_unmet_titles_returns_episode_scoped_rows_for_series() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            r#""cutoff-series""#,
        )
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile(
            "cutoff-series",
            "1080P",
        )])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Cutoff Episodes".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
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

    let file_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Cutoff Episodes/Season 01/Cutoff Episodes - S01E01.mkv"
                .to_string(),
            size_bytes: 1_000,
            quality_label: Some("720p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    media_files
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link media file to episode");

    let items = app
        .list_cutoff_unmet_titles(&user, Some(MediaFacet::Series), None)
        .await
        .expect("cutoff unmet query should succeed");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title_id, title.id);
    assert_eq!(items[0].episode_id.as_deref(), Some(episode.id.as_str()));
    assert_eq!(items[0].current_tier, "720P");
    assert_eq!(items[0].target_tier, "1080P");
}

#[tokio::test]
async fn list_cutoff_unmet_titles_skips_legacy_titles_with_stale_profile_tags() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            r#""cutoff-global""#,
        )
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile(
            "cutoff-global",
            "720P",
        )])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, titles) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Stale Tag".into(),
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
    titles
        .store
        .lock()
        .await
        .iter_mut()
        .find(|stored| stored.id == title.id)
        .expect("stored title")
        .tags
        .push("scryer:quality-profile:missing-profile".to_string());

    media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Stale Tag.mkv".to_string(),
            size_bytes: 1_000,
            quality_label: Some("480p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");

    let items = app
        .list_cutoff_unmet_titles(&user, Some(MediaFacet::Movie), None)
        .await
        .expect("cutoff unmet query should succeed");

    assert!(items.is_empty());
}

#[tokio::test]
async fn search_titles_supports_facet_filter() {
    let (app, user) = bootstrap();

    app.add_title(
        &user,
        NewTitle {
            name: "Movie A".into(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,

            ..Default::default()
        },
    )
    .await
    .expect("create movie");

    app.add_title(
        &user,
        NewTitle {
            name: "Show B".into(),
            facet: MediaFacet::Series,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,

            ..Default::default()
        },
    )
    .await
    .expect("create series");

    let tvs = app
        .list_titles_unpaged(&user, Some(MediaFacet::Series), None, None)
        .await
        .expect("list titles");

    assert!(tvs.iter().all(|item| item.facet == MediaFacet::Series));
}

#[tokio::test]
async fn search_indexers_for_title_keeps_direct_nab_searches_uncategorized_when_routing_is_empty() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let recording_client = Arc::new(RecordingCategoriesIndexerClient::new(
        "Generic.Release.2026.1080p.WEB-DL",
    ));
    let (app, user) =
        bootstrap_with_search_settings_and_indexer(settings, recording_client.clone());

    let movie = app
        .add_title(
            &user,
            NewTitle {
                name: "Default Category Movie".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2026),
                ..Default::default()
            },
        )
        .await
        .expect("create movie title");
    let series = app
        .add_title(
            &user,
            NewTitle {
                name: "Default Category Series".into(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    let anime = app
        .add_title(
            &user,
            NewTitle {
                name: "Default Category Anime".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");

    app.search_indexers_for_title(
        &user,
        movie.id.clone(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("movie search should succeed");
    app.search_indexers_for_title(
        &user,
        series.id.clone(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("series search should succeed");
    app.search_indexers_for_title(
        &user,
        anime.id.clone(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("anime search should succeed");

    let calls = recording_client.calls.lock().await.clone();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].newznab_categories, None);
    assert_eq!(calls[1].newznab_categories, None);
    assert_eq!(calls[2].newznab_categories, None);
}

#[tokio::test]
async fn search_indexers_for_episode_dedupes_equivalent_structured_series_queries() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let recording_client = Arc::new(RecordingStructuredQueryIndexerClient::default());
    let (app, user) = bootstrap_with_search_settings_indexer_and_configs(
        settings,
        recording_client.clone(),
        vec![synthetic_direct_nab_indexer_config("idx-series", "nzbgeek")],
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Synthetic Signal".into(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");

    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "2".to_string(),
            label: Some("Season 2".to_string()),
            ordered_path: None,
            narrative_order: Some("2".to_string()),
            first_episode_number: Some("11".to_string()),
            last_episode_number: Some("11".to_string()),
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
            episode_number: Some("11".to_string()),
            season_number: Some("2".to_string()),
            episode_label: Some("S02E11".to_string()),
            title: Some("Episode 11".to_string()),
            air_date: Some("2026-01-01".to_string()),
            duration_seconds: Some(1_500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: Some("tvdb-series-211".to_string()),
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create series episode");

    app.search_indexers_for_episode(
        &user,
        title.id.clone(),
        "2".to_string(),
        "11".to_string(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("series episode search should succeed");

    let calls = recording_client.calls.lock().await.clone();
    assert_eq!(
        calls,
        vec![RecordedStructuredQueryCall {
            query: "Synthetic Signal S02E11".to_string(),
            season: Some(2),
            episode: Some(11),
            absolute_episode: None,
        }]
    );
}

#[tokio::test]
async fn search_indexers_for_episode_dedupes_equivalent_structured_anime_queries() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let recording_client = Arc::new(RecordingStructuredQueryIndexerClient::default());
    let (app, user) = bootstrap_with_search_settings_indexer_and_configs(
        settings,
        recording_client.clone(),
        vec![synthetic_direct_nab_indexer_config("idx-anime", "nzbgeek")],
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Synthetic Atlas".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");

    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "2".to_string(),
            label: Some("Season 2".to_string()),
            ordered_path: None,
            narrative_order: Some("2".to_string()),
            first_episode_number: Some("11".to_string()),
            last_episode_number: Some("11".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create anime season");

    app.services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("11".to_string()),
            season_number: Some("2".to_string()),
            episode_label: Some("S02E11".to_string()),
            title: Some("Episode 11".to_string()),
            air_date: Some("2026-01-01".to_string()),
            duration_seconds: Some(1_500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("35".to_string()),
            overview: None,
            tvdb_id: Some("tvdb-anime-211".to_string()),
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create anime episode");

    app.search_indexers_for_episode(
        &user,
        title.id.clone(),
        "2".to_string(),
        "11".to_string(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("anime episode search should succeed");

    let calls = recording_client.calls.lock().await.clone();
    assert_eq!(
        calls,
        vec![RecordedStructuredQueryCall {
            query: "Synthetic Atlas 035".to_string(),
            season: Some(2),
            episode: Some(11),
            absolute_episode: Some(35),
        }]
    );
}

#[tokio::test]
async fn search_indexers_anime_required_english_accepts_dual_audio_release() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(
        "Anime.Show.S01E01.1080p.WEB-DL.DUAL.H.265",
    ));
    let (app, user) = bootstrap_with_search_settings_and_indexer(settings, indexer_client);

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

    app.set_facet_required_audio_languages(&user, "anime", vec!["English".to_string()])
        .await
        .expect("set anime required audio");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Anime Show".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");

    let results = app
        .search_indexers_for_title(
            &user,
            title.id.clone(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("search indexers for title");

    assert_eq!(results.len(), 1);
    let parsed = results[0]
        .parsed_release_metadata
        .as_ref()
        .expect("search result should be parsed");
    assert_eq!(
        parsed.languages_audio,
        vec!["eng".to_string(), "jpn".to_string()]
    );
    let decision = results[0]
        .quality_profile_decision
        .as_ref()
        .expect("search result should be scored");
    assert!(decision.allowed);
    assert!(
        decision
            .scoring_log
            .iter()
            .any(|entry| entry.code == "required_audio_languages_match")
    );
}

#[tokio::test]
async fn search_indexers_for_title_uses_tagged_aliases_for_auto_evaluation() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(
        "Nightfall.Heavy.Metal.Dark.Fantasy.S01E01.1080p.NF.WEB-DL",
    ));
    let (app, user) = bootstrap_with_search_settings_and_indexer(settings, indexer_client);

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

    let search_user = create_user_with_permissions(
        &app,
        &user,
        "title_search_user",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await
    .expect("create search user");
    let search_token = app
        .issue_access_token(&search_user)
        .await
        .expect("issue search token");
    let authed_search_user = app
        .authenticate_token(&search_token)
        .await
        .expect("authenticate search user");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Nightfall!!".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "1309".to_string(),
                }],
                year: Some(2022),
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    app.services
        .catalog
        .titles
        .update_title_hydrated_metadata(
            &title.id,
            TitleMetadataUpdate {
                tagged_aliases: vec![scryer_domain::TaggedAlias {
                    name: "Nightfall Heavy Metal Dark Fantasy".to_string(),
                    language: "eng".to_string(),
                }],
                ..Default::default()
            },
        )
        .await
        .expect("persist tagged aliases");

    let results = app
        .search_indexers_for_title(
            &authed_search_user,
            title.id.clone(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("search indexers for title");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].auto_eligible, Some(true));
    assert_eq!(results[0].auto_decision_code.as_deref(), Some("eligible"));
    assert!(results[0].candidate_token.is_some());
}

#[tokio::test]
async fn search_indexers_for_title_returns_results_when_candidate_token_attachment_fails() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(
        "Failure.Recovery.2026.1080p.WEB-DL",
    ));
    let (app, user) = bootstrap_with_search_settings_and_indexer(settings, indexer_client);

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
                name: "Failure Recovery".into(),
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

    let mut ghost_actor = User {
        id: "ghost-search-user".to_string(),
        username: "ghost".to_string(),
        password_hash: None,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    ghost_actor.authorization = scryer_domain::UserAuthorization {
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::ManageTitles,
        ]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };

    let results = app
        .search_indexers_for_title(
            &ghost_actor,
            title.id.clone(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("search should still succeed without candidate signing key");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].candidate_token, None);
}

#[tokio::test]
async fn list_cutoff_unmet_titles_page_bounds_and_total() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY, r#""p720""#)
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile("p720", "720p")])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());

    // Three monitored movies, each with a below-cutoff (480p vs 720p) file.
    for name in ["Alpha", "Bravo", "Charlie"] {
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: name.into(),
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
        media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: format!("/library/{name}.mkv"),
                size_bytes: 1_000,
                quality_label: Some("480p".to_string()),
                ..Default::default()
            })
            .await
            .expect("insert media file");
    }

    // First page of 2 of 3, with the full total reported.
    let page = app
        .list_cutoff_unmet_titles_page(&user, Some(MediaFacet::Movie), None, 2, 0)
        .await
        .expect("paged cutoff query should succeed");
    assert_eq!(page.total, 3);
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].title_name, "Alpha");
    assert_eq!(page.items[1].title_name, "Bravo");

    // Second page: remainder.
    let page = app
        .list_cutoff_unmet_titles_page(&user, Some(MediaFacet::Movie), None, 2, 2)
        .await
        .expect("paged cutoff query should succeed");
    assert_eq!(page.total, 3);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].title_name, "Charlie");

    // limit == 0 returns just the total.
    let page = app
        .list_cutoff_unmet_titles_page(&user, Some(MediaFacet::Movie), None, 0, 0)
        .await
        .expect("paged cutoff query should succeed");
    assert_eq!(page.total, 3);
    assert!(page.items.is_empty());
}
