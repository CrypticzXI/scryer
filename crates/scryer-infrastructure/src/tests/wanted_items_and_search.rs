use super::*;

#[tokio::test]
async fn complete_wanted_item_for_title_updates_matching_row_in_one_step() {
    let db = std::env::temp_dir().join(format!(
        "scryer_complete_wanted_item_for_title_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = wanted_store(&services);
    let catalog = title_store(&services);
    let now = Utc::now().to_rfc3339();

    let title = make_test_title("title-series", None);
    TitleRepository::create(&catalog, title)
        .await
        .expect("title should insert");

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, media_type, status,
          current_score, grabbed_release, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("wanted-episode")
    .bind("title-series")
    .bind("movie")
    .bind("wanted")
    .bind(42i64)
    .bind("Existing Release")
    .bind(&now)
    .bind(&now)
    .execute(services.pool())
    .await
    .expect("wanted item should insert");

    let completed = workflow
        .complete_wanted_item_for_title("title-series", None, Some("2026-04-20T00:00:00Z"), None)
        .await
        .expect("completion should succeed");

    assert!(completed);

    let row = sqlx::query(
        "SELECT status, last_search_at, current_score, grabbed_release
         FROM wanted_items
         WHERE id = ?",
    )
    .bind("wanted-episode")
    .fetch_one(services.pool())
    .await
    .expect("wanted item should load");

    assert_eq!(row.get::<String, _>("status"), "completed");
    assert_eq!(
        row.get::<Option<String>, _>("last_search_at"),
        Some("2026-04-20T00:00:00Z".to_string())
    );
    assert_eq!(row.get::<Option<i64>, _>("current_score"), Some(42));
    assert_eq!(
        row.get::<Option<String>, _>("grabbed_release"),
        Some("Existing Release".to_string())
    );

    sqlx::query("UPDATE wanted_items SET status = ?, grabbed_release = ? WHERE id = ?")
        .bind("wanted")
        .bind("Stale Grabbed Release")
        .bind("wanted-episode")
        .execute(services.pool())
        .await
        .expect("wanted item should reset for scored completion");

    workflow
        .complete_wanted_item_for_title(
            "title-series",
            None,
            Some("2026-04-20T01:00:00Z"),
            Some(720),
        )
        .await
        .expect("scored completion should succeed");

    let row = sqlx::query(
        "SELECT current_score, grabbed_release
         FROM wanted_items
         WHERE id = ?",
    )
    .bind("wanted-episode")
    .fetch_one(services.pool())
    .await
    .expect("wanted item should load after scored completion");

    assert_eq!(row.get::<Option<i64>, _>("current_score"), Some(720));
    assert_eq!(row.get::<Option<String>, _>("grabbed_release"), None);

    sqlx::query(
        "UPDATE wanted_items SET status = ?, current_score = ?, grabbed_release = ? WHERE id = ?",
    )
    .bind("wanted")
    .bind(720i64)
    .bind("Negative Score Release")
    .bind("wanted-episode")
    .execute(services.pool())
    .await
    .expect("wanted item should reset for negative scored completion");

    workflow
        .complete_wanted_item_for_title(
            "title-series",
            None,
            Some("2026-04-20T02:00:00Z"),
            Some(-15),
        )
        .await
        .expect("negative scored completion should succeed");

    let row = sqlx::query(
        "SELECT current_score, grabbed_release
         FROM wanted_items
         WHERE id = ?",
    )
    .bind("wanted-episode")
    .fetch_one(services.pool())
    .await
    .expect("wanted item should load after negative scored completion");

    assert_eq!(row.get::<Option<i64>, _>("current_score"), Some(-15));
    assert_eq!(row.get::<Option<String>, _>("grabbed_release"), None);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn list_wanted_items_filters_on_latest_decision_code() {
    let (services, db) = temp_services("scryer_wanted_latest_decision").await;
    let workflow = wanted_store(&services);
    let catalog = title_store(&services);
    let now = Utc::now();

    let title = make_test_title("title-latest-decision", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    let other_title = make_test_title("title-latest-decision-other", None);
    TitleRepository::create(&catalog, other_title.clone())
        .await
        .expect("other title should insert");

    let wanted_mismatch = WantedItem {
        id: "wanted-mismatch".to_string(),
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: None,
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    };
    let wanted_quality_blocked = WantedItem {
        id: "wanted-quality-blocked".to_string(),
        title_id: other_title.id.clone(),
        title_name: Some(other_title.name.clone()),
        ..wanted_mismatch.clone()
    };

    workflow
        .upsert_wanted_item(&wanted_mismatch)
        .await
        .expect("first wanted item should insert");
    workflow
        .upsert_wanted_item(&wanted_quality_blocked)
        .await
        .expect("second wanted item should insert");

    workflow
        .insert_release_decision(&ReleaseDecision {
            id: "decision-1".to_string(),
            wanted_item_id: wanted_mismatch.id.clone(),
            title_id: title.id.clone(),
            release_title: "Mismatch Release".to_string(),
            release_url: None,
            release_size_bytes: None,
            decision_code: "title_mismatch".to_string(),
            candidate_score: 0,
            current_score: None,
            score_delta: None,
            explanation_json: None,
            created_at: now.to_rfc3339(),
        })
        .await
        .expect("mismatch decision should insert");
    workflow
        .insert_release_decision(&ReleaseDecision {
            id: "decision-2".to_string(),
            wanted_item_id: wanted_quality_blocked.id.clone(),
            title_id: other_title.id.clone(),
            release_title: "Old Mismatch Release".to_string(),
            release_url: None,
            release_size_bytes: None,
            decision_code: "title_mismatch".to_string(),
            candidate_score: 0,
            current_score: None,
            score_delta: None,
            explanation_json: None,
            created_at: (now - chrono::Duration::minutes(2)).to_rfc3339(),
        })
        .await
        .expect("older mismatch decision should insert");
    workflow
        .insert_release_decision(&ReleaseDecision {
            id: "decision-3".to_string(),
            wanted_item_id: wanted_quality_blocked.id.clone(),
            title_id: other_title.id.clone(),
            release_title: "New Blocked Release".to_string(),
            release_url: None,
            release_size_bytes: None,
            decision_code: "quality_blocked".to_string(),
            candidate_score: 0,
            current_score: None,
            score_delta: None,
            explanation_json: None,
            created_at: now.to_rfc3339(),
        })
        .await
        .expect("latest blocked decision should insert");

    let items = workflow
        .list_wanted_items(WantedItemsQuery {
            latest_decision_codes: vec!["title_mismatch".into()],
            limit: 50,
            ..WantedItemsQuery::default()
        })
        .await
        .expect("filtered wanted items should load");
    let count = workflow
        .count_wanted_items(WantedItemsQuery {
            latest_decision_codes: vec!["title_mismatch".into()],
            ..WantedItemsQuery::default()
        })
        .await
        .expect("filtered wanted count should load");

    assert_eq!(items.len(), 1);
    assert_eq!(count, 1);
    assert_eq!(items[0].id, wanted_mismatch.id);
    assert!(items[0].mismatch_recovery_eligible);
    let latest_decision = items[0]
        .latest_release_decision
        .as_ref()
        .expect("latest decision should be hydrated");
    assert_eq!(latest_decision.decision_code, "title_mismatch");
    assert_eq!(latest_decision.release_title, "Mismatch Release");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_search_matches_aliases_slug_and_typos_with_direct_priority() {
    let (services, db) = temp_services("scryer_catalog_title_search").await;
    let catalog = title_store(&services);

    let mut direct_title = make_test_title("title-search-direct", None);
    direct_title.name = "Schoolhouse Rock! Earth".to_string();
    direct_title.slug = Some("schoolhouse-rock-earth".to_string());
    direct_title.aliases = vec!["School House Rock".to_string()];
    direct_title.tagged_aliases = vec![TaggedAlias {
        name: "Schoolhouse Planet Earth".to_string(),
        language: "eng".to_string(),
    }];
    TitleRepository::create(&catalog, direct_title.clone())
        .await
        .expect("direct title should insert");

    let mut typo_title = make_test_title("title-search-typo", None);
    typo_title.name = "Schoolhouze Rock Earth".to_string();
    TitleRepository::create(&catalog, typo_title.clone())
        .await
        .expect("typo title should insert");

    let alias_hits = TitleRepository::list(&catalog, None, Some("school house rock".to_string()))
        .await
        .expect("alias search should load");
    assert_eq!(
        alias_hits.first().map(|title| title.id.as_str()),
        Some(direct_title.id.as_str())
    );

    let slug_hits =
        TitleRepository::list(&catalog, None, Some("schoolhouse rock earth".to_string()))
            .await
            .expect("slug search should load");
    assert_eq!(
        slug_hits.first().map(|title| title.id.as_str()),
        Some(direct_title.id.as_str())
    );

    let typo_hits =
        TitleRepository::list(&catalog, None, Some("scholhouse rock earth".to_string()))
            .await
            .expect("typo search should load");
    assert_eq!(
        typo_hits.first().map(|title| title.id.as_str()),
        Some(direct_title.id.as_str())
    );
    assert!(typo_hits.iter().any(|title| title.id == typo_title.id));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_search_short_typo_does_not_return_loose_spellfix_neighbors() {
    let (services, db) = temp_services("scryer_catalog_title_search_short_typo").await;
    let catalog = title_store(&services);

    let mut aoashi = make_test_title("title-search-aoashi", None);
    aoashi.name = "Aoashi".to_string();
    aoashi.facet = MediaFacet::Anime;
    TitleRepository::create(&catalog, aoashi.clone())
        .await
        .expect("close typo target should insert");

    let mut ranma = make_test_title("title-search-ranma", None);
    ranma.name = "Ranma 1/2 (2024)".to_string();
    ranma.facet = MediaFacet::Anime;
    TitleRepository::create(&catalog, ranma.clone())
        .await
        .expect("loose neighbor should insert");

    let mut blue_box = make_test_title("title-search-blue-box", None);
    blue_box.name = "Blue Box".to_string();
    blue_box.facet = MediaFacet::Anime;
    TitleRepository::create(&catalog, blue_box.clone())
        .await
        .expect("loose neighbor should insert");

    let mut her_blue_sky = make_test_title("title-search-her-blue-sky", None);
    her_blue_sky.name = "Her Blue Sky".to_string();
    TitleRepository::create(&catalog, her_blue_sky.clone())
        .await
        .expect("movie loose neighbor should insert");

    let hits = TitleRepository::list(&catalog, None, Some("aashi".to_string()))
        .await
        .expect("short typo search should load");
    let hit_ids = hits
        .into_iter()
        .map(|title| title.id)
        .collect::<HashSet<_>>();

    assert!(hit_ids.contains(&aoashi.id));
    assert!(!hit_ids.contains(&ranma.id));
    assert!(!hit_ids.contains(&blue_box.id));
    assert!(!hit_ids.contains(&her_blue_sky.id));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_search_returns_valid_single_substitution_typo_for_frieren() {
    let (services, db) = temp_services("scryer_catalog_title_search_frieren_typo").await;
    let catalog = title_store(&services);

    let mut frieren = make_test_title("title-search-frieren", None);
    frieren.name = "Silver Horizon: Beyond Journey's End".to_string();
    frieren.facet = MediaFacet::Anime;
    frieren.aliases = vec!["Sora no Vale".to_string(), "Frieren".to_string()];
    TitleRepository::create(&catalog, frieren.clone())
        .await
        .expect("frieren should insert");

    let mut friend = make_test_title("title-search-friend", None);
    friend.name = "Friend".to_string();
    TitleRepository::create(&catalog, friend.clone())
        .await
        .expect("friend should insert");

    let mut firefly = make_test_title("title-search-firefly", None);
    firefly.name = "Signal Run".to_string();
    TitleRepository::create(&catalog, firefly.clone())
        .await
        .expect("firefly should insert");

    let hits = TitleRepository::list(&catalog, None, Some("friefen".to_string()))
        .await
        .expect("frieren typo search should load");

    assert_eq!(
        hits.first().map(|title| title.id.as_str()),
        Some(frieren.id.as_str())
    );
    assert!(!hits.iter().any(|title| title.id == friend.id));
    assert!(!hits.iter().any(|title| title.id == firefly.id));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_search_projection_refreshes_after_hydrated_metadata_update_and_delete() {
    let (services, db) = temp_services("scryer_title_search_projection_refresh").await;
    let catalog = title_store(&services);

    let mut title = make_test_title("title-projection-refresh", None);
    title.name = "Example Show".to_string();
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let missing_hits = TitleRepository::list(&catalog, None, Some("earth defenders".to_string()))
        .await
        .expect("pre-update search should load");
    assert!(missing_hits.is_empty());

    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            slug: Some("earth-defenders".to_string()),
            aliases: vec!["Earth's Defenders".to_string()],
            tagged_aliases: vec![TaggedAlias {
                name: "Earth Defenders".to_string(),
                language: "eng".to_string(),
            }],
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            ..Default::default()
        },
    )
    .await
    .expect("hydrated metadata should update");

    let alias_hits = TitleRepository::list(&catalog, None, Some("earth defenders".to_string()))
        .await
        .expect("alias search should load");
    assert_eq!(
        alias_hits
            .first()
            .map(|match_title| match_title.id.as_str()),
        Some(title.id.as_str())
    );

    TitleRepository::delete(&catalog, &title.id)
        .await
        .expect("title should delete");

    let deleted_hits = TitleRepository::list(&catalog, None, Some("earth defenders".to_string()))
        .await
        .expect("post-delete search should load");
    assert!(deleted_hits.is_empty());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn list_wanted_items_filters_with_fuzzy_title_search() {
    let (services, db) = temp_services("scryer_wanted_title_search").await;
    let workflow = wanted_store(&services);
    let catalog = title_store(&services);
    let now = Utc::now();

    let mut title = make_test_title("title-search-match", None);
    title.name = "Schoolhouse Rock! Earth".to_string();
    title.aliases = vec!["School House Rock".to_string()];
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("matching title should insert");
    let mut other_title = make_test_title("title-search-other", None);
    other_title.name = "Different Show".to_string();
    TitleRepository::create(&catalog, other_title.clone())
        .await
        .expect("other title should insert");

    let wanted_match = WantedItem {
        id: "wanted-search-match".to_string(),
        title_id: title.id.clone(),
        title_name: Some("Schoolhouse Rock! Earth".to_string()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "episode".to_string(),
        last_search_at: None,
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    };
    let wanted_other = WantedItem {
        id: "wanted-search-other".to_string(),
        title_id: other_title.id.clone(),
        title_name: Some("Different Show".to_string()),
        ..wanted_match.clone()
    };

    workflow
        .upsert_wanted_item(&wanted_match)
        .await
        .expect("matching wanted item should insert");
    workflow
        .upsert_wanted_item(&wanted_other)
        .await
        .expect("other wanted item should insert");

    let items = workflow
        .list_wanted_items(WantedItemsQuery {
            title_search: Some("scholhouse erth".into()),
            limit: 50,
            ..WantedItemsQuery::default()
        })
        .await
        .expect("filtered wanted items should load");
    let count = workflow
        .count_wanted_items(WantedItemsQuery {
            title_search: Some("scholhouse erth".into()),
            ..WantedItemsQuery::default()
        })
        .await
        .expect("filtered wanted count should load");

    assert_eq!(items.len(), 1);
    assert_eq!(count, 1);
    assert_eq!(items[0].id, wanted_match.id);

    let short_items = workflow
        .list_wanted_items(WantedItemsQuery {
            title_search: Some("roc".into()),
            limit: 50,
            ..WantedItemsQuery::default()
        })
        .await
        .expect("short filtered wanted items should load");
    let short_count = workflow
        .count_wanted_items(WantedItemsQuery {
            title_search: Some("roc".into()),
            ..WantedItemsQuery::default()
        })
        .await
        .expect("short filtered wanted count should load");

    assert_eq!(short_items.len(), 1);
    assert_eq!(short_count, 1);
    assert_eq!(short_items[0].id, wanted_match.id);

    let short_title_hits = TitleRepository::list(&catalog, None, Some("roc".to_string()))
        .await
        .expect("short title list search should load");
    assert_eq!(short_title_hits.len(), 1);
    assert_eq!(short_title_hits[0].id, title.id);

    let _ = std::fs::remove_file(db);
}
