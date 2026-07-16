use super::*;
use crate::media::images::normalize_title_image_source_url;

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
    let expected_w250_url = format!(
        "/images/titles/title-priority-poster/poster/w250?v={}",
        test_title_image_version("11111111111111111111111111111111")
    );
    assert_eq!(
        updated_poster.poster_url.as_deref(),
        Some(expected_w250_url.as_str())
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
        Some(expected_w250_url.as_str())
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn normalized_title_image_urls_advance_to_the_next_variant_after_one_refresh() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_normalized_source_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);
    let requested_source_url = "/banners/posters/normalized-source.jpg";
    let normalized_source_url =
        normalize_title_image_source_url(requested_source_url).expect("source should normalize");
    let title = make_test_title("title-normalized-image-source", Some(requested_source_url));
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let first = title_images
        .list_title_image_refresh_work(1, &[])
        .await
        .expect("initial refresh work should list");
    assert_eq!(first.len(), 1);
    assert_variant_target(&first[0], TitleImageKind::Poster, "w250");

    let digest = format!("blake3:{}", blake3::hash(&[4, 5, 6]).to_hex());
    let mut result = test_title_image_source_result(
        TitleImageKind::Poster,
        &normalized_source_url,
        "w250",
        250,
        375,
        &digest,
    );
    result.requested_source_url = requested_source_url.to_string();
    title_images
        .upsert_title_image_source_result(&title.id, result, None)
        .await
        .expect("normalized image result should insert");

    let persisted_source_url: String =
        sqlx::query_scalar("SELECT poster_url FROM titles WHERE id = ?")
            .bind(&title.id)
            .fetch_one(&services.pool)
            .await
            .expect("title source should load");
    assert_eq!(persisted_source_url, normalized_source_url);
    let next = title_images
        .list_title_image_refresh_work(1, &[])
        .await
        .expect("next refresh work should list");
    assert_eq!(next.len(), 1);
    assert_variant_target(&next[0], TitleImageKind::Poster, "w70");

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_image_blobs_deduplicate_and_gc_only_after_last_reference() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_blob_gc_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);
    let housekeeping = housekeeping_store(&services);
    let shared_digest = format!("blake3:{}", blake3::hash(&[4, 5, 6]).to_hex());

    for (id, variant_key) in [
        ("title-shared-image-a", "w250"),
        ("title-shared-image-b", "w2048"),
    ] {
        let title = make_test_title(id, Some("https://tvdb.example/shared-poster.jpg"));
        TitleRepository::create(&catalog, title.clone())
            .await
            .expect("title should insert");
        title_images
            .upsert_title_image_source_result(
                &title.id,
                test_title_image_source_result_with_variants(
                    TitleImageKind::Poster,
                    "https://tvdb.example/shared-poster.jpg",
                    vec![test_title_image_variant_record(
                        variant_key,
                        250,
                        375,
                        &shared_digest,
                    )],
                ),
                None,
            )
            .await
            .expect("title image should insert");
    }

    let blob_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_blobs")
        .fetch_one(&services.pool)
        .await
        .expect("blob count should load");
    let variant_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_variants")
        .fetch_one(&services.pool)
        .await
        .expect("variant count should load");
    assert_eq!(blob_count, 1);
    assert_eq!(variant_count, 2);
    assert_eq!(
        HousekeepingRepository::prune_unreferenced_title_image_blobs(&housekeeping, 100)
            .await
            .expect("referenced blob prune should succeed"),
        0
    );

    sqlx::query("DELETE FROM title_images WHERE title_id = ?")
        .bind("title-shared-image-a")
        .execute(&services.pool)
        .await
        .expect("first image reference should delete");
    assert_eq!(
        HousekeepingRepository::prune_unreferenced_title_image_blobs(&housekeeping, 100)
            .await
            .expect("shared blob prune should succeed"),
        0
    );

    sqlx::query("DELETE FROM title_images WHERE title_id = ?")
        .bind("title-shared-image-b")
        .execute(&services.pool)
        .await
        .expect("last image reference should delete");
    assert_eq!(
        HousekeepingRepository::prune_unreferenced_title_image_blobs(&housekeeping, 100)
            .await
            .expect("unreferenced blob prune should succeed"),
        1
    );

    for (digest, byte) in [("orphan-a", 1_u8), ("orphan-b", 2_u8)] {
        sqlx::query(
            "INSERT INTO title_image_blobs (
                digest, format, width, height, bytes, created_at, updated_at
             ) VALUES (?, 'avif', 1, 1, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
        )
        .bind(digest)
        .bind(vec![byte])
        .execute(&services.pool)
        .await
        .expect("orphan blob should insert");
    }
    assert_eq!(
        HousekeepingRepository::prune_unreferenced_title_image_blobs(&housekeeping, 1)
            .await
            .expect("bounded orphan prune should succeed"),
        1
    );
    let remaining_blob_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_blobs")
        .fetch_one(&services.pool)
        .await
        .expect("remaining blob count should load");
    assert_eq!(remaining_blob_count, 1);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_image_blob_upsert_rejects_forged_digest_without_creating_references() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_forged_digest_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);
    let source_url = "https://tvdb.example/forged-poster.jpg";
    let title = make_test_title("title-forged-image-digest", Some(source_url));
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let mut result = test_title_image_source_result(
        TitleImageKind::Poster,
        source_url,
        "w250",
        250,
        375,
        "forged-digest-payload",
    );
    result.variants[0].digest =
        "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string();
    let error = title_images
        .upsert_title_image_source_result(&title.id, result, None)
        .await
        .expect_err("forged digest must fail");
    assert!(error.to_string().contains("digest"));

    let image_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_images")
        .fetch_one(&services.pool)
        .await
        .expect("image count should load");
    let variant_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_variants")
        .fetch_one(&services.pool)
        .await
        .expect("variant count should load");
    let blob_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_blobs")
        .fetch_one(&services.pool)
        .await
        .expect("blob count should load");
    assert_eq!((image_count, variant_count, blob_count), (0, 0, 0));

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_image_blob_upsert_rejects_conflicting_blob_metadata() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_conflicting_blob_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);
    let source_url = "https://tvdb.example/conflicting-poster.jpg";
    let title = make_test_title("title-conflicting-image-blob", Some(source_url));
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    let result = test_title_image_source_result(
        TitleImageKind::Poster,
        source_url,
        "w250",
        250,
        375,
        "conflicting-blob-payload",
    );
    let variant = &result.variants[0];
    sqlx::query(
        "INSERT INTO title_image_blobs (
            digest, format, width, height, bytes, created_at, updated_at
         ) VALUES (?, 'png', 1, 1, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
    )
    .bind(&variant.digest)
    .bind(&variant.bytes)
    .execute(&services.pool)
    .await
    .expect("conflicting blob should insert");

    let error = title_images
        .upsert_title_image_source_result(&title.id, result, None)
        .await
        .expect_err("conflicting blob metadata must fail");
    assert!(error.to_string().contains("conflict"));

    let image_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_images")
        .fetch_one(&services.pool)
        .await
        .expect("image count should load");
    let variant_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_variants")
        .fetch_one(&services.pool)
        .await
        .expect("variant count should load");
    assert_eq!((image_count, variant_count), (0, 0));

    drop(services);
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
    let blob_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_blobs")
        .fetch_one(&services.pool)
        .await
        .expect("blob count should load");
    assert_eq!(image_count, 0);
    assert_eq!(variant_count, 0);
    assert_eq!(blob_count, 0);

    let _ = std::fs::remove_file(db);
}
