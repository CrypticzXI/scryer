use super::*;

fn hydration_test_movie(tvdb_id: i64, name: &str) -> MovieMetadata {
    MovieMetadata {
        target_key: None,
        smg_id: None,
        primary_source: "tvdb".to_string(),
        tvdb_id: Some(tvdb_id),
        name: name.to_string(),
        slug: name.to_ascii_lowercase().replace(' ', "-"),
        year: Some(2026),
        content_status: "Released".to_string(),
        overview: format!("{name} overview"),
        poster_url: format!("https://example.invalid/{tvdb_id}.jpg"),
        background_url: None,
        language: "eng".to_string(),
        original_language: Some("eng".to_string()),
        runtime_minutes: 90,
        sort_title: name.to_string(),
        imdb_id: format!("tt{tvdb_id:07}"),
        tmdb_id: None,
        popularity: None,
        anidb_id: None,
        canonical_tags: vec![],
        studio: "Scryer Studios".to_string(),
        tmdb_release_date: Some("2026-01-01".to_string()),
        ratings: Default::default(),
        credits: Vec::new(),
    }
}

fn hydration_test_title(name: &str, tvdb_id: i64) -> NewTitle {
    NewTitle {
        name: name.to_string(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![ExternalId {
            source: "tvdb".to_string(),
            value: tvdb_id.to_string(),
        }],
        min_availability: None,
        ..Default::default()
    }
}

async fn wait_for_title_metadata(app: &AppUseCase, user: &User, title_id: &str) -> Title {
    timeout(Duration::from_secs(2), async {
        loop {
            let titles = app
                .list_titles_unpaged(user, Some(MediaFacet::Movie), None, None)
                .await
                .expect("titles should load");
            if let Some(title) = titles.into_iter().find(|title| title.id == title_id)
                && title.metadata_fetched_at.is_some()
            {
                return title;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("title metadata should hydrate")
}

async fn assert_title_metadata_pending(app: &AppUseCase, user: &User, title_id: &str) {
    let titles = app
        .list_titles_unpaged(user, Some(MediaFacet::Movie), None, None)
        .await
        .expect("titles should load");
    let title = titles
        .into_iter()
        .find(|title| title.id == title_id)
        .expect("title should exist");
    assert_eq!(title.metadata_fetched_at, None);
}

async fn stop_title_hydration_worker(
    token: tokio_util::sync::CancellationToken,
    handle: tokio::task::JoinHandle<()>,
) {
    token.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("title hydration worker should stop")
        .expect("title hydration worker should not panic");
}

#[derive(Default)]
struct MovieTitleResolutionGateway {
    unsupported: bool,
    redirected_from: Option<i64>,
    calls: Mutex<Vec<(Vec<MovieTitleRef>, bool)>>,
}

#[async_trait]
impl MetadataGateway for MovieTitleResolutionGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Err(AppError::Repository(
            "not used by identity backfill tests".into(),
        ))
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Err(AppError::Repository(
            "not used by identity backfill tests".into(),
        ))
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Err(AppError::Repository(
            "not used by identity backfill tests".into(),
        ))
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Err(AppError::Repository(
            "not used by identity backfill tests".into(),
        ))
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        Err(AppError::Repository(
            "not used by identity backfill tests".into(),
        ))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::Repository(
            "not used by identity backfill tests".into(),
        ))
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Err(AppError::Repository(
            "not used by identity backfill tests".into(),
        ))
    }

    async fn resolve_movie_titles(
        &self,
        refs: &[MovieTitleRef],
        create_missing: bool,
    ) -> AppResult<Vec<TitleResolution>> {
        self.calls
            .lock()
            .await
            .push((refs.to_vec(), create_missing));
        if self.unsupported {
            return Err(AppError::Repository(
                "metadata gateway does not support title-id queries".into(),
            ));
        }

        Ok(refs
            .iter()
            .enumerate()
            .filter_map(|(ref_index, reference)| {
                reference.tvdb_id.map(|tvdb_id| TitleResolution {
                    ref_index,
                    resolved: true,
                    smg_id: Some(tvdb_id + 1_000_000),
                    kind: "movie".to_string(),
                    primary_source: "tvdb".to_string(),
                    redirected_from: self.redirected_from,
                    created: false,
                    external_ids: vec![],
                    reason: String::new(),
                })
            })
            .collect())
    }
}

#[tokio::test]
async fn movie_smg_identity_backfill_links_ids_and_resumes_from_its_cursor() {
    let gateway = Arc::new(MovieTitleResolutionGateway::default());
    let (app, user, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    app.add_title_with_outcome(&user, hydration_test_title("Cursor A", 951_001))
        .await
        .expect("first title should be created");
    app.add_title_with_outcome(&user, hydration_test_title("Cursor B", 951_002))
        .await
        .expect("second title should be created");
    let token = tokio_util::sync::CancellationToken::new();

    let first =
        crate::catalog::title_hydration::run_movie_smg_identity_backfill_tick(&app, &token, 1)
            .await;
    let crate::catalog::title_hydration::MovieSmgIdentityBackfillTick::Completed(summary) = first
    else {
        panic!("first backfill tick should complete");
    };
    assert_eq!(summary.linked, 1);
    assert_eq!(
        titles
            .store
            .lock()
            .await
            .iter()
            .filter(|title| {
                title
                    .external_ids
                    .iter()
                    .any(|external_id| external_id.source == "smg")
            })
            .count(),
        1
    );

    let second =
        crate::catalog::title_hydration::run_movie_smg_identity_backfill_tick(&app, &token, 1)
            .await;
    let crate::catalog::title_hydration::MovieSmgIdentityBackfillTick::Completed(summary) = second
    else {
        panic!("second backfill tick should complete");
    };
    assert_eq!(summary.linked, 1);
    assert!(titles.store.lock().await.iter().all(|title| {
        title
            .external_ids
            .iter()
            .any(|external_id| external_id.source == "smg")
    }));

    let calls = gateway.calls.lock().await;
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|(_, create_missing)| !create_missing));
    assert!(calls.iter().all(|(refs, _)| refs.len() == 1));
}

#[tokio::test]
async fn movie_smg_identity_backfill_skips_the_default_not_supported_gateway_error() {
    let gateway = Arc::new(MovieTitleResolutionGateway {
        unsupported: true,
        ..Default::default()
    });
    let (app, user, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    app.add_title_with_outcome(&user, hydration_test_title("Unsupported", 951_003))
        .await
        .expect("title should be created");

    let token = tokio_util::sync::CancellationToken::new();
    let tick =
        crate::catalog::title_hydration::run_movie_smg_identity_backfill_tick(&app, &token, 1)
            .await;
    assert!(matches!(
        tick,
        crate::catalog::title_hydration::MovieSmgIdentityBackfillTick::NotSupported
    ));
    assert!(titles.store.lock().await.iter().all(|title| {
        title
            .external_ids
            .iter()
            .all(|external_id| !external_id.source.eq_ignore_ascii_case("smg"))
    }));
}

#[tokio::test]
async fn prompt_title_hydration_worker_processes_pending_title_after_wake() {
    let (app, user) = bootstrap();
    let tvdb_id = 901_001;
    let app = app.with_test_overrides(|services| {
        services.with_metadata_gateway(Arc::new(MockMetadataGateway {
            movies: HashMap::from([(tvdb_id, hydration_test_movie(tvdb_id, "Wake Movie"))]),
        }))
    });
    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_title_hydration_loop(
        app.clone(),
        token.clone(),
    ));

    let outcome = app
        .add_title_with_outcome(&user, hydration_test_title("Wake Movie", tvdb_id))
        .await
        .expect("add title should succeed");
    assert_eq!(
        outcome.metadata_hydration_state,
        AddTitleHydrationState::Pending
    );

    let hydrated = wait_for_title_metadata(&app, &user, &outcome.title.id).await;
    assert_eq!(hydrated.name, "Wake Movie");
    assert_eq!(hydrated.year, Some(2026));
    assert_eq!(hydrated.language.as_deref(), Some("eng"));
    assert_eq!(hydrated.metadata_language.as_deref(), Some("eng"));

    stop_title_hydration_worker(token, handle).await;
}

#[tokio::test]
async fn prompt_title_hydration_worker_yields_to_active_scan_facet() {
    let (app, user) = bootstrap();
    let tvdb_id = 901_002;
    let app = app.with_test_overrides(|services| {
        services.with_metadata_gateway(Arc::new(MockMetadataGateway {
            movies: HashMap::from([(tvdb_id, hydration_test_movie(tvdb_id, "Scan Blocked Movie"))]),
        }))
    });
    let scan = app
        .runtime
        .library
        .library_scan_tracker
        .start_session(MediaFacet::Movie)
        .await
        .expect("scan should start");
    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_title_hydration_loop(
        app.clone(),
        token.clone(),
    ));

    let outcome = app
        .add_title_with_outcome(&user, hydration_test_title("Scan Blocked Movie", tvdb_id))
        .await
        .expect("add title should succeed");
    sleep(Duration::from_millis(75)).await;
    assert_title_metadata_pending(&app, &user, &outcome.title.id).await;

    app.runtime
        .library
        .library_scan_tracker
        .fail_session(&scan.session_id)
        .await
        .expect("scan should finish");
    let hydrated = wait_for_title_metadata(&app, &user, &outcome.title.id).await;
    assert_eq!(hydrated.name, "Scan Blocked Movie");

    stop_title_hydration_worker(token, handle).await;
}
