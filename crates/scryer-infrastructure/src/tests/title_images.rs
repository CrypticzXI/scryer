use super::*;

#[tokio::test]
async fn title_image_refresh_work_uses_global_variant_priorities() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_priority_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let poster_source = "https://tmdb.example/poster.jpg";
    let poster = make_test_title("title-priority-poster", Some(poster_source));
    TitleRepository::create(&catalog, poster.clone())
        .await
        .expect("poster title should insert");

    let fanart = make_test_title("title-priority-fanart", None);
    TitleRepository::create(&catalog, fanart.clone())
        .await
        .expect("fanart title should insert");
    sqlx::query("UPDATE titles SET background_url = ? WHERE id = ?")
        .bind("https://tmdb.example/fanart.jpg")
        .bind(&fanart.id)
        .execute(&services.pool)
        .await
        .expect("fanart source should update");

    let first = title_images
        .list_title_image_refresh_work(10, &[])
        .await
        .expect("priority work should list");
    assert_eq!(first[0].title_id, poster.id);
    assert_variant_target(&first[0], TitleImageKind::Poster, "w250");

    title_images
        .upsert_title_image_source_result(
            &poster.id,
            test_title_image_source_result(
                TitleImageKind::Poster,
                poster_source,
                "w250",
                250,
                375,
                "11111111111111111111111111111111",
            ),
            None,
        )
        .await
        .expect("w250 should upsert");
    let updated_poster = TitleRepository::get_by_id(&catalog, &poster.id)
        .await
        .expect("poster title should load")
        .expect("poster title should exist");
    assert_eq!(
        updated_poster.poster_url.as_deref(),
        Some("/images/titles/title-priority-poster/poster/w250?v=1111111111111111")
    );

    let second = title_images
        .list_title_image_refresh_work(10, &[])
        .await
        .expect("priority work should list");
    assert_eq!(second[0].title_id, poster.id);
    assert_variant_target(&second[0], TitleImageKind::Poster, "w70");

    title_images
        .upsert_title_image_source_result(
            &poster.id,
            test_title_image_source_result(
                TitleImageKind::Poster,
                poster_source,
                "w70",
                70,
                105,
                "22222222222222222222222222222222",
            ),
            None,
        )
        .await
        .expect("w70 should upsert");

    let third = title_images
        .list_title_image_refresh_work(10, &[])
        .await
        .expect("priority work should list");
    assert_eq!(third[0].title_id, fanart.id);
    assert_variant_target(&third[0], TitleImageKind::Fanart, "w1280");

    title_images
        .upsert_title_image_source_result(
            &fanart.id,
            test_title_image_source_result(
                TitleImageKind::Fanart,
                "https://tmdb.example/fanart.jpg",
                "w1280",
                1280,
                720,
                "33333333333333333333333333333333",
            ),
            None,
        )
        .await
        .expect("w1280 should upsert");

    let fourth = title_images
        .list_title_image_refresh_work(10, &[])
        .await
        .expect("priority work should list");
    assert_eq!(fourth[0].title_id, poster.id);
    assert_variant_target(&fourth[0], TitleImageKind::Poster, "w500");

    title_images
        .upsert_title_image_source_result(
            &poster.id,
            test_title_image_source_result(
                TitleImageKind::Poster,
                poster_source,
                "w500",
                500,
                750,
                "44444444444444444444444444444444",
            ),
            None,
        )
        .await
        .expect("w500 should upsert");
    let updated_poster = TitleRepository::get_by_id(&catalog, &poster.id)
        .await
        .expect("poster title should load")
        .expect("poster title should exist");
    assert_eq!(
        updated_poster.poster_url.as_deref(),
        Some("/images/titles/title-priority-poster/poster/w250?v=1111111111111111")
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_image_refresh_work_skips_failed_image_sets_for_current_pass() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_skip_current_pass_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let first = make_test_title(
        "title-skip-current-pass-1",
        Some("https://tmdb.example/poster-1.jpg"),
    );
    let second = make_test_title(
        "title-skip-current-pass-2",
        Some("https://tmdb.example/poster-2.jpg"),
    );
    TitleRepository::create(&catalog, first.clone())
        .await
        .expect("first title should insert");
    TitleRepository::create(&catalog, second.clone())
        .await
        .expect("second title should insert");

    let initial = title_images
        .list_title_image_refresh_work(1, &[])
        .await
        .expect("initial work should list");
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].title_id, first.id);
    assert_variant_target(&initial[0], TitleImageKind::Poster, "w250");

    let skipped = initial.clone();
    let next = title_images
        .list_title_image_refresh_work(1, &skipped)
        .await
        .expect("next work should list");
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].title_id, second.id);
    assert_variant_target(&next[0], TitleImageKind::Poster, "w250");

    let retry_on_next_pass = title_images
        .list_title_image_refresh_work(1, &[])
        .await
        .expect("retry work should list");
    assert_eq!(retry_on_next_pass.len(), 1);
    assert_eq!(retry_on_next_pass[0].title_id, first.id);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_update_metadata_preserves_provider_image_url_after_local_image_projection() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_provider_preserve_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let source_url = "https://tvdb.example/provider-poster.jpg";
    let title = make_test_title("title-provider-preserve", Some(source_url));
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    title_images
        .upsert_title_image_source_result(
            &title.id,
            test_title_image_source_result_with_variants(
                TitleImageKind::Poster,
                source_url,
                vec![test_title_image_variant_record(
                    "w250",
                    250,
                    375,
                    "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
                )],
            ),
            None,
        )
        .await
        .expect("title image should insert");

    let updated = TitleRepository::update_metadata(
        &catalog,
        &title.id,
        None,
        None,
        Some(vec!["favorite".to_string()]),
        None,
    )
    .await
    .expect("title metadata should update");
    assert_eq!(updated.poster_source_url.as_deref(), Some(source_url));
    assert!(
        updated
            .poster_url
            .as_deref()
            .is_some_and(|url| url.starts_with("/images/titles/"))
    );

    let row = sqlx::query("SELECT poster_url, poster_local_path FROM titles WHERE id = ?")
        .bind(&title.id)
        .fetch_one(&services.pool)
        .await
        .expect("title row should load");
    let stored_source: Option<String> = row.get("poster_url");
    let stored_local_path: Option<String> = row.get("poster_local_path");
    assert_eq!(stored_source.as_deref(), Some(source_url));
    assert!(
        stored_local_path
            .as_deref()
            .is_some_and(|url| url.starts_with("/images/titles/"))
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_artwork_url_update_clears_stale_local_paths_for_changed_sources() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_source_invalidation_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let poster_source = "https://tvdb.example/poster-old.jpg";
    let background_source = "https://tvdb.example/background-old.jpg";
    let mut title = make_test_title("title-source-invalidation", Some(poster_source));
    title.background_url = Some(background_source.to_string());
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    title_images
        .upsert_title_image_source_result(
            &title.id,
            test_title_image_source_result_with_variants(
                TitleImageKind::Poster,
                poster_source,
                vec![test_title_image_variant_record(
                    "w250",
                    250,
                    375,
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                )],
            ),
            None,
        )
        .await
        .expect("poster image should insert");
    title_images
        .upsert_title_image_source_result(
            &title.id,
            test_title_image_source_result_with_variants(
                TitleImageKind::Fanart,
                background_source,
                vec![test_title_image_variant_record(
                    "w1280",
                    1280,
                    720,
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                )],
            ),
            None,
        )
        .await
        .expect("fanart image should insert");

    let new_poster_source = "https://image.tmdb.org/t/p/w500/poster-new.jpg";
    let changed = catalog
        .update_title_artwork_urls(&[TitleArtworkUrlUpdate {
            title_id: title.id.clone(),
            poster_url: Some(new_poster_source.to_string()),
            background_url: Some(background_source.to_string()),
        }])
        .await
        .expect("poster source update should apply");
    assert_eq!(changed, 1);

    let row = sqlx::query(
        "SELECT poster_url, poster_local_path, background_url, background_local_path
           FROM titles
          WHERE id = ?",
    )
    .bind(&title.id)
    .fetch_one(&services.pool)
    .await
    .expect("title row should load after poster source update");
    let stored_poster: Option<String> = row.get("poster_url");
    let stored_poster_local: Option<String> = row.get("poster_local_path");
    let stored_background: Option<String> = row.get("background_url");
    let stored_background_local: Option<String> = row.get("background_local_path");
    assert_eq!(stored_poster.as_deref(), Some(new_poster_source));
    assert_eq!(stored_poster_local, None);
    assert_eq!(stored_background.as_deref(), Some(background_source));
    assert!(
        stored_background_local
            .as_deref()
            .is_some_and(|url| url.starts_with("/images/titles/"))
    );

    let new_background_source = "https://image.tmdb.org/t/p/w1280/background-new.jpg";
    let changed = catalog
        .update_title_artwork_urls(&[TitleArtworkUrlUpdate {
            title_id: title.id.clone(),
            poster_url: Some(new_poster_source.to_string()),
            background_url: Some(new_background_source.to_string()),
        }])
        .await
        .expect("background source update should apply");
    assert_eq!(changed, 1);

    let row = sqlx::query(
        "SELECT poster_url, poster_local_path, background_url, background_local_path
           FROM titles
          WHERE id = ?",
    )
    .bind(&title.id)
    .fetch_one(&services.pool)
    .await
    .expect("title row should load after background source update");
    let stored_poster: Option<String> = row.get("poster_url");
    let stored_poster_local: Option<String> = row.get("poster_local_path");
    let stored_background: Option<String> = row.get("background_url");
    let stored_background_local: Option<String> = row.get("background_local_path");
    assert_eq!(stored_poster.as_deref(), Some(new_poster_source));
    assert_eq!(stored_poster_local, None);
    assert_eq!(stored_background.as_deref(), Some(new_background_source));
    assert_eq!(stored_background_local, None);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_image_refresh_work_ignores_local_title_image_routes() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_local_route_refresh_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title(
        "title-local-route-refresh",
        Some("/images/titles/title-local-route-refresh/poster/w500?v=deadbeef"),
    );
    TitleRepository::create(&catalog, title)
        .await
        .expect("title should insert");

    let upstream = make_test_title(
        "title-http-route-segment-refresh",
        Some("https://cdn.example/images/titles/upstream-poster.jpg"),
    );
    TitleRepository::create(&catalog, upstream.clone())
        .await
        .expect("upstream title should insert");

    let pending = title_images
        .list_title_image_refresh_work(10, &[])
        .await
        .expect("list pending poster refresh should succeed");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].title_id, upstream.id);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn clear_title_image_cache_repairs_polluted_urls_and_clears_db_cache() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_cache_clear_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let source_url = "https://tvdb.example/cache-clear-poster.jpg";
    let repaired = make_test_title("title-cache-clear-repair", Some(source_url));
    TitleRepository::create(&catalog, repaired.clone())
        .await
        .expect("repair title should insert");
    title_images
        .upsert_title_image_source_result(
            &repaired.id,
            test_title_image_source_result_with_variants(
                TitleImageKind::Poster,
                source_url,
                vec![test_title_image_variant_record(
                    "w250",
                    250,
                    375,
                    "ffffffffffffffffffffffffffffffff",
                )],
            ),
            None,
        )
        .await
        .expect("title image should insert");
    sqlx::query("UPDATE titles SET poster_url = ? WHERE id = ?")
        .bind("/images/titles/title-cache-clear-repair/poster/w250?v=ffffffffffffffff")
        .bind(&repaired.id)
        .execute(&services.pool)
        .await
        .expect("polluted source should update");

    let unrecoverable = make_test_title(
        "title-cache-clear-unrecoverable",
        Some("/images/titles/title-cache-clear-unrecoverable/poster/w500?v=badbadbad"),
    );
    TitleRepository::create(&catalog, unrecoverable.clone())
        .await
        .expect("unrecoverable title should insert");

    title_images
        .clear_title_image_cache()
        .await
        .expect("title image cache should clear");

    let repaired_row = sqlx::query("SELECT poster_url, poster_local_path FROM titles WHERE id = ?")
        .bind(&repaired.id)
        .fetch_one(&services.pool)
        .await
        .expect("repaired row should load");
    let repaired_source: Option<String> = repaired_row.get("poster_url");
    let repaired_local_path: Option<String> = repaired_row.get("poster_local_path");
    assert_eq!(repaired_source.as_deref(), Some(source_url));
    assert!(repaired_local_path.is_none());

    let unrecoverable_row = sqlx::query(
        "SELECT poster_url, metadata_hydration_next_attempt_at FROM titles WHERE id = ?",
    )
    .bind(&unrecoverable.id)
    .fetch_one(&services.pool)
    .await
    .expect("unrecoverable row should load");
    let unrecoverable_source: Option<String> = unrecoverable_row.get("poster_url");
    let next_attempt: Option<String> = unrecoverable_row.get("metadata_hydration_next_attempt_at");
    assert!(unrecoverable_source.is_none());
    assert!(next_attempt.is_some());

    let image_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_images")
        .fetch_one(&services.pool)
        .await
        .expect("image count should load");
    let variant_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_variants")
        .fetch_one(&services.pool)
        .await
        .expect("variant count should load");
    assert_eq!(image_count, 0);
    assert_eq!(variant_count, 0);

    let _ = std::fs::remove_file(db);
}
