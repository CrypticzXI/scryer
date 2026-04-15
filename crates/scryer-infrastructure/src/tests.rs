use super::*;
use chrono::Utc;
use scryer_application::{
    CollectionUpdate, DownloadSubmissionRepository, EpisodeUpdate, InsertMediaFileInput,
    LibraryScanUnmatchedItem, LibraryScanUnmatchedItemRepository,
    LibraryScanUnmatchedSearchAttempt, MediaFileRepository, ShowRepository, TitleImageBlob,
    TitleImageKind, TitleImageReplacement, TitleImageRepository, TitleImageStorageMode,
    TitleImageVariantRecord, TitleRepository, UserRepository,
};
use scryer_domain::{
    Collection, CollectionType, Entitlement, Episode, ExternalId, InterstitialMovieMetadata,
    MediaFacet, Title,
};
use sqlx::{Row, sqlite::SqlitePoolOptions};

#[tokio::test]
async fn sqlite_can_initialize() {
    let db = std::env::temp_dir().join(format!(
        "scryer_store_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy()).await.unwrap();
    let users = UserRepository::list_all(&catalog_store(&services))
        .await
        .expect("query should return users after initialization");

    assert!(!users.is_empty());
    let _ = std::fs::remove_file(db);
}

fn make_test_title(id: &str, poster_url: Option<&str>) -> Title {
    Title {
        id: id.to_string(),
        name: "Poster Test".to_string(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: Utc::now(),
        year: Some(2026),
        overview: Some("overview".to_string()),
        poster_url: poster_url.map(str::to_string),
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        genres: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    }
}

fn catalog_store(services: &SqliteServices) -> SqliteCatalogStore {
    SqliteCatalogStore::new(services)
}

fn library_state_store(services: &SqliteServices) -> SqliteLibraryStateStore {
    SqliteLibraryStateStore::new(services)
}

#[tokio::test]
async fn nzbget_client_is_sendable() {
    let client = NzbgetDownloadClient::new(
        "http://127.0.0.1:6789".to_string(),
        Some("user".into()),
        Some("pass".into()),
        "SCORE".to_string(),
    );
    // We only validate that it can be built and is callable in type system.
    let _ = client.endpoint();
}

#[tokio::test]
async fn title_queries_prefer_local_cached_poster_url() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = catalog_store(&services);
    let library_state = library_state_store(&services);

    let title = make_test_title("title-1", Some("https://tvdb.example/poster.jpg"));
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let before_cache = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        before_cache.poster_url.as_deref(),
        Some("https://tvdb.example/poster.jpg")
    );

    library_state
        .replace_title_image(
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Poster,
                source_url: "https://tvdb.example/poster.jpg".to_string(),
                source_etag: Some("\"etag-1\"".to_string()),
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 1000,
                source_height: 1500,
                storage_mode: TitleImageStorageMode::AvifMaster,
                master_format: "avif".to_string(),
                master_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                master_width: 1000,
                master_height: 1500,
                master_bytes: vec![1, 2, 3],
                variants: vec![TitleImageVariantRecord {
                    variant_key: "w500".to_string(),
                    format: "avif".to_string(),
                    width: 500,
                    height: 750,
                    bytes: vec![7, 8, 9],
                    sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
                }],
            },
        )
        .await
        .expect("title image should insert");

    let after_cache = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        after_cache.poster_url.as_deref(),
        Some("/images/titles/title-1/poster/w500?v=bbbbbbbbbbbbbbbb")
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_change_local_version_when_cached_poster_changes() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_version_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = catalog_store(&services);
    let library_state = library_state_store(&services);

    let title = make_test_title("title-2", Some("https://tvdb.example/poster-a.jpg"));
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    for (source_url, sha) in [
        (
            "https://tvdb.example/poster-a.jpg",
            "11111111111111111111111111111111",
        ),
        (
            "https://tvdb.example/poster-b.jpg",
            "22222222222222222222222222222222",
        ),
    ] {
        library_state
            .replace_title_image(
                &title.id,
                TitleImageReplacement {
                    kind: TitleImageKind::Poster,
                    source_url: source_url.to_string(),
                    source_etag: None,
                    source_last_modified: None,
                    source_format: "jpeg".to_string(),
                    source_width: 1000,
                    source_height: 1500,
                    storage_mode: TitleImageStorageMode::AvifMaster,
                    master_format: "avif".to_string(),
                    master_sha256: sha.to_string(),
                    master_width: 1000,
                    master_height: 1500,
                    master_bytes: vec![1, 2, 3],
                    variants: vec![TitleImageVariantRecord {
                        variant_key: "w500".to_string(),
                        format: "avif".to_string(),
                        width: 500,
                        height: 750,
                        bytes: vec![7, 8, 9],
                        sha256: sha.to_string(),
                    }],
                },
            )
            .await
            .expect("title image should upsert");

        sqlx::query("UPDATE titles SET poster_url = ? WHERE id = ?")
            .bind(source_url)
            .bind(&title.id)
            .execute(&services.pool)
            .await
            .expect("source url should update");
    }

    let updated = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        updated.poster_url.as_deref(),
        Some("/images/titles/title-2/poster/w500?v=2222222222222222")
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_find_by_external_id() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_external_id_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = catalog_store(&services);

    let mut title = make_test_title("title-external-id", None);
    title.external_ids = vec![ExternalId {
        source: "TVDB".to_string(),
        value: "123456".to_string(),
    }];
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let found = catalog
        .find_by_external_id("tvdb", "123456")
        .await
        .expect("lookup should succeed")
        .expect("title should exist");

    assert_eq!(found.id, title.id);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn media_file_source_signature_refresh_preserves_scan_status() {
    let db = std::env::temp_dir().join(format!(
        "scryer_media_file_signature_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = catalog_store(&services);
    let library_state = library_state_store(&services);

    let title = make_test_title("title-media-file", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let file_id = library_state
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Movie.Title.2024.mkv".to_string(),
            size_bytes: 4_096,
            ..Default::default()
        })
        .await
        .expect("media file should insert");

    sqlx::query("UPDATE media_files SET scan_status = 'scanned' WHERE id = ?")
        .bind(&file_id)
        .execute(&services.pool)
        .await
        .expect("scan status should update");

    library_state
        .update_media_file_source_signature(
            &file_id,
            4_096,
            Some("unix_mtime_nsec_v1".to_string()),
            Some("1:2".to_string()),
        )
        .await
        .expect("source signature should refresh");

    let media_file = library_state
        .get_media_file_by_id(&file_id)
        .await
        .expect("lookup should succeed")
        .expect("media file should exist");

    assert_eq!(media_file.scan_status, "scanned");
    assert_eq!(
        media_file.source_signature_scheme.as_deref(),
        Some("unix_mtime_nsec_v1")
    );
    assert_eq!(media_file.source_signature_value.as_deref(), Some("1:2"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_use_local_original_url_for_original_storage_mode() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_original_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = catalog_store(&services);
    let library_state = library_state_store(&services);

    let title = make_test_title("title-3", Some("https://tvdb.example/poster-original.jpg"));
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    library_state
        .replace_title_image(
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Poster,
                source_url: "https://tvdb.example/poster-original.jpg".to_string(),
                source_etag: None,
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 400,
                source_height: 600,
                storage_mode: TitleImageStorageMode::Original,
                master_format: "jpeg".to_string(),
                master_sha256: "cccccccccccccccccccccccccccccccc".to_string(),
                master_width: 400,
                master_height: 600,
                master_bytes: vec![3, 2, 1],
                variants: Vec::new(),
            },
        )
        .await
        .expect("title image should insert");

    let updated = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        updated.poster_url.as_deref(),
        Some("/images/titles/title-3/poster/original?v=cccccccccccccccc")
    );

    let original = library_state
        .get_title_image_blob(&title.id, TitleImageKind::Poster, "original")
        .await
        .expect("original blob lookup should succeed");
    assert_eq!(
        original,
        Some(TitleImageBlob {
            content_type: "image/jpeg".to_string(),
            etag: "cccccccccccccccccccccccccccccccc".to_string(),
            bytes: vec![3, 2, 1],
        })
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_fall_back_to_original_when_w500_variant_is_missing() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_incomplete_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = catalog_store(&services);
    let library_state = library_state_store(&services);

    let title = make_test_title(
        "title-4",
        Some("https://tvdb.example/poster-incomplete.jpg"),
    );
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    library_state
        .replace_title_image(
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Poster,
                source_url: "https://tvdb.example/poster-incomplete.jpg".to_string(),
                source_etag: None,
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 1000,
                source_height: 1500,
                storage_mode: TitleImageStorageMode::AvifMaster,
                master_format: "avif".to_string(),
                master_sha256: "dddddddddddddddddddddddddddddddd".to_string(),
                master_width: 1000,
                master_height: 1500,
                master_bytes: vec![9, 8, 7],
                variants: Vec::new(),
            },
        )
        .await
        .expect("title image should insert");

    let updated = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        updated.poster_url.as_deref(),
        Some("/images/titles/title-4/poster/original?v=dddddddddddddddd")
    );

    let pending = library_state
        .list_titles_requiring_image_refresh(TitleImageKind::Poster, 10)
        .await
        .expect("list pending poster refresh should succeed");
    assert!(
        pending.iter().any(|task| task.title_id == title.id),
        "incomplete AVIF cache rows should be re-queued for repair"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_validate_mode_rejects_pending_schema() {
    let db = std::env::temp_dir().join(format!(
        "scryer_validate_mode_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let result =
        SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::ValidateOnly).await;
    assert!(
        result.is_err(),
        "validate mode should reject unapplied migrations"
    );
    let err = match result {
        Ok(_) => panic!("validate mode should reject unapplied migrations"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("pending migration"));
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_bootstrap_rejects_unknown_or_newer_schema_history() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_compat_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let _ = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    let too_new_key = "999999_too_new";
    sqlx::query(
        "UPDATE _sqlx_migrations
            SET checksum = ?
          WHERE version = ?",
    )
    .bind(Vec::<u8>::new())
    .bind(1i64)
    .execute(&pool)
    .await
    .expect("tamper first migration checksum");
    sqlx::query(
        "INSERT INTO _sqlx_migrations
        (version, description, installed_on, success, checksum, execution_time)
        VALUES (?, ?, CURRENT_TIMESTAMP, 1, ?, 0)",
    )
    .bind(999999i64)
    .bind(too_new_key)
    .bind(Vec::<u8>::new())
    .execute(&pool)
    .await
    .expect("insert new migration");

    let result = SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::Apply).await;
    assert!(result.is_err());
    let err = match result {
        Ok(_) => panic!("bad migration history should fail compatibility check"),
        Err(err) => err,
    };

    let message = err.to_string();
    assert!(message.contains("checksum mismatch"));
    assert!(message.contains("migrations newer than supported"));
    assert!(message.contains("Please update scryer"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_bootstrap_accepts_known_legacy_prod_checksums() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_legacy_checksum_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let _ = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    sqlx::query(
        "UPDATE _sqlx_migrations
            SET checksum = ?
          WHERE version = ?",
    )
    .bind(hex_to_bytes(
        "c9866f8bfd780ddb1213bd5101afbf9335a223755152c45a2aeb29998489ee6604105481161751e1213024c56116087a",
    ))
    .bind(51i64)
    .execute(&pool)
    .await
    .expect("set legacy checksum for migration 51");

    sqlx::query(
        "UPDATE _sqlx_migrations
            SET checksum = ?
          WHERE version = ?",
    )
    .bind(hex_to_bytes(
        "ffbf3f5a3b3207a257887c63bda5465216703964ac8544e7cf0fcd2064e155b269c0ff24a0b587de480c3e23264d038f",
    ))
    .bind(63i64)
    .execute(&pool)
    .await
    .expect("set legacy checksum for migration 63");

    let _ = SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::ValidateOnly)
        .await
        .expect("legacy prod checksums should remain upgrade-compatible");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn specials_convergence_migration_repoints_legacy_season_zero_references() {
    let db = std::env::temp_dir().join(format!(
        "scryer_specials_convergence_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let _ = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO titles (id, name, name_normalized, facet, monitored, status, tags, external_ids, created_at)
         VALUES (?, ?, ?, ?, 1, 'active', '[]', '[]', ?)",
    )
    .bind("title-series")
    .bind("Legacy Series")
    .bind("legacy series")
    .bind("series")
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert title");

    sqlx::query(
        "INSERT INTO collections
         (id, title_id, collection_type, collection_index, label, monitored, created_at, special_movies_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("legacy-specials")
    .bind("title-series")
    .bind("season")
    .bind("0")
    .bind("Season 0")
    .bind(0i64)
    .bind(&now)
    .bind("[]")
    .execute(&pool)
    .await
    .expect("insert legacy specials");

    sqlx::query(
        "INSERT INTO collections
         (id, title_id, collection_type, collection_index, label, monitored, created_at, special_movies_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("canonical-specials")
    .bind("title-series")
    .bind("specials")
    .bind("0")
    .bind("Specials")
    .bind(0i64)
    .bind(&now)
    .bind("[]")
    .execute(&pool)
    .await
    .expect("insert canonical specials");

    sqlx::query(
        "INSERT INTO episodes
         (id, title_id, collection_id, episode_type, episode_number, season_number, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("episode-legacy")
    .bind("title-series")
    .bind("legacy-specials")
    .bind("special")
    .bind("1")
    .bind("0")
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy episode");

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, media_type, search_phase, status, created_at, updated_at, collection_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("wanted-legacy")
    .bind("title-series")
    .bind("episode")
    .bind("primary")
    .bind("wanted")
    .bind(&now)
    .bind(&now)
    .bind("legacy-specials")
    .execute(&pool)
    .await
    .expect("insert legacy wanted item");

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, media_type, search_phase, status, created_at, updated_at, collection_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("wanted-canonical")
    .bind("title-series")
    .bind("episode")
    .bind("primary")
    .bind("wanted")
    .bind(&now)
    .bind(&now)
    .bind("canonical-specials")
    .execute(&pool)
    .await
    .expect("insert canonical wanted item");

    let migration_sql =
        include_str!("../../scryer/src/db/migrations/0070_specials_collection_convergence.sql");
    for statement in migration_sql
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
    {
        sqlx::query(statement)
            .execute(&pool)
            .await
            .expect("run migration statement");
    }

    let collections: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, collection_type FROM collections WHERE title_id = ? ORDER BY id",
    )
    .bind("title-series")
    .fetch_all(&pool)
    .await
    .expect("load collections");
    assert_eq!(
        collections,
        vec![("canonical-specials".to_string(), "specials".to_string())]
    );

    let episode_collection: String =
        sqlx::query_scalar("SELECT collection_id FROM episodes WHERE id = ?")
            .bind("episode-legacy")
            .fetch_one(&pool)
            .await
            .expect("load migrated episode collection");
    assert_eq!(episode_collection, "canonical-specials");

    let wanted_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM wanted_items WHERE collection_id = ? ORDER BY id")
            .bind("canonical-specials")
            .fetch_all(&pool)
            .await
            .expect("load wanted items");
    assert_eq!(wanted_ids, vec!["wanted-canonical".to_string()]);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migrations_apply_then_validate_is_idempotent() {
    let db = std::env::temp_dir().join(format!(
        "scryer_validate_then_apply_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy()).await.unwrap();
    drop(services);

    let _ = SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::ValidateOnly)
        .await
        .expect("applied DB should pass validate mode");

    let _ = std::fs::remove_file(db);
}

fn hex_to_bytes(input: &str) -> Vec<u8> {
    assert_eq!(input.len() % 2, 0, "hex input must have even length");

    input
        .as_bytes()
        .chunks(2)
        .map(|chunk| {
            let pair = std::str::from_utf8(chunk).expect("hex bytes should be utf8");
            u8::from_str_radix(pair, 16).expect("hex pair should decode")
        })
        .collect()
}

#[tokio::test]
async fn tracked_state_upsert_creates_download_submission_row_when_missing() {
    use crate::queries::workflow::get_tracked_state_query;

    let db = std::env::temp_dir().join(format!(
        "scryer_tracked_state_upsert_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow_store = SqliteWorkflowStore::new(&services);

    workflow_store
        .update_tracked_state("weaver", "job-123", "failed")
        .await
        .expect("tracked state upsert should succeed without a preexisting submission row");

    let tracked_state = get_tracked_state_query(services.pool(), "weaver", "job-123")
        .await
        .expect("tracked state query should succeed");
    assert_eq!(tracked_state.as_deref(), Some("failed"));

    let row = sqlx::query(
        "SELECT title_id, facet FROM download_submissions WHERE download_client_type = ? AND download_client_item_id = ?",
    )
    .bind("weaver")
    .bind("job-123")
    .fetch_one(services.pool())
    .await
    .expect("download submission row should exist");

    let title_id: String = row.get("title_id");
    let facet: String = row.get("facet");
    assert!(title_id.is_empty());
    assert!(facet.is_empty());

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn unique_constraints_enforce_settings_and_user_entitlements() {
    let db = std::env::temp_dir().join(format!(
        "scryer_unique_constraints_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let _ = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO settings_definitions
        (id, category, scope, key_name, data_type, default_value_json, is_sensitive, validation_json, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("sd-settings")
    .bind("app")
    .bind("global")
    .bind("theme")
    .bind("string")
    .bind("{}")
    .bind(0)
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert settings definition");

    sqlx::query(
        "INSERT INTO settings_values
        (id, setting_definition_id, scope, scope_id, value_json, source, updated_by_user_id, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("sv-1")
    .bind("sd-settings")
    .bind("global")
    .bind(Option::<String>::None)
    .bind("{}",)
    .bind("seed")
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert first settings value");

    let duplicate_setting_value = sqlx::query(
        "INSERT INTO settings_values
        (id, setting_definition_id, scope, scope_id, value_json, source, updated_by_user_id, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("sv-2")
    .bind("sd-settings")
    .bind("global")
    .bind(Option::<String>::None)
    .bind("{}",)
    .bind("seed")
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await;
    assert!(duplicate_setting_value.is_err());

    sqlx::query("INSERT INTO users (id, username, entitlements) VALUES (?, ?, ?)")
        .bind("user-1")
        .bind("constraint_user")
        .bind("[]")
        .execute(&pool)
        .await
        .expect("insert user");

    sqlx::query("INSERT INTO entitlements (code, description, category) VALUES (?, ?, ?)")
        .bind("ent.code.manage")
        .bind("Manage")
        .bind("admin")
        .execute(&pool)
        .await
        .expect("insert entitlement");

    sqlx::query(
        "INSERT INTO user_entitlements (user_id, entitlement_code, granted_by_user_id, granted_at, expires_at)
        VALUES (?, ?, ?, ?, ?)",
    )
    .bind("user-1")
    .bind("ent.code.manage")
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(Option::<String>::None)
    .execute(&pool)
    .await
    .expect("insert first user entitlement");

    let duplicate_user_entitlement = sqlx::query(
        "INSERT INTO user_entitlements (user_id, entitlement_code, granted_by_user_id, granted_at, expires_at)
        VALUES (?, ?, ?, ?, ?)",
    )
    .bind("user-1")
    .bind("ent.code.manage")
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(Option::<String>::None)
    .execute(&pool)
    .await;
    assert!(duplicate_user_entitlement.is_err());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn user_crud_queries_work() {
    let db = std::env::temp_dir().join(format!(
        "scryer_user_queries_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = catalog_store(&services);

    let created = UserRepository::create(
        &catalog,
        scryer_domain::User {
            id: "u-1".to_string(),
            username: "editor".to_string(),
            entitlements: vec![Entitlement::ViewCatalog],
            password_hash: None,
        },
    )
    .await
    .expect("create user");

    let from_db = UserRepository::get_by_id(&catalog, &created.id)
        .await
        .expect("query by id")
        .expect("id should exist");
    assert_eq!(from_db.username, created.username);

    let updated = UserRepository::update_entitlements(
        &catalog,
        &created.id,
        vec![Entitlement::ManageTitle, Entitlement::ViewHistory],
    )
    .await
    .expect("update entitlements");
    assert!(updated.entitlements.contains(&Entitlement::ManageTitle));

    UserRepository::delete(&catalog, &created.id)
        .await
        .expect("delete user");
    let missing = UserRepository::get_by_id(&catalog, &created.id)
        .await
        .expect("query after delete");
    assert!(missing.is_none());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn sqlite_show_queries_roundtrip() {
    let db = std::env::temp_dir().join(format!(
        "scryer_show_roundtrip_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy()).await.unwrap();
    let catalog = catalog_store(&services);

    let title = Title {
        id: "title-show-1".into(),
        name: "Sample Show".into(),
        facet: MediaFacet::Series,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: Utc::now(),
        year: None,
        overview: None,
        poster_url: None,
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        genres: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    };
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("insert title");

    let collection = Collection {
        id: "collection-show-1".into(),
        title_id: title.id.clone(),
        collection_type: CollectionType::Season,
        collection_index: "1".into(),
        label: Some("Season One".into()),
        ordered_path: None,
        narrative_order: Some("1".into()),
        first_episode_number: Some("1".into()),
        last_episode_number: Some("12".into()),
        interstitial_movie: Some(InterstitialMovieMetadata {
            tvdb_id: "12345".into(),
            name: "Test Movie".into(),
            slug: "test-movie".into(),
            year: Some(2024),
            content_status: "released".into(),
            overview: "Interstitial overview".into(),
            poster_url: "https://example.com/poster.jpg".into(),
            language: "eng".into(),
            runtime_minutes: 97,
            sort_title: "Test Movie".into(),
            imdb_id: "tt1234567".into(),
            genres: vec!["Action".into(), "Anime".into()],
            studio: "Studio Test".into(),
            digital_release_date: Some("2024-01-01".into()),
            association_confidence: Some("high".into()),
            continuity_status: Some("canon".into()),
            movie_form: Some("movie".into()),
            confidence: Some("high".into()),
            signal_summary: Some("TVDB marked special as critical to story".into()),
            placement: Some("ordered".into()),
            movie_tmdb_id: Some("99001".into()),
            movie_mal_id: Some("5001".into()),
            movie_anidb_id: None,
        }),
        specials_movies: vec![InterstitialMovieMetadata {
            tvdb_id: "67890".into(),
            name: "Recap Movie".into(),
            slug: "recap-movie".into(),
            year: Some(2014),
            content_status: "released".into(),
            overview: "Recap of the first half.".into(),
            poster_url: "https://example.com/recap.jpg".into(),
            language: "eng".into(),
            runtime_minutes: 90,
            sort_title: "Recap Movie".into(),
            imdb_id: "tt7654321".into(),
            genres: vec!["Action".into()],
            studio: "Studio Test".into(),
            digital_release_date: Some("2014-11-01".into()),
            association_confidence: Some("high".into()),
            continuity_status: Some("unknown".into()),
            movie_form: Some("recap".into()),
            confidence: Some("high".into()),
            signal_summary: Some("TVDB special category marks this as a recap".into()),
            placement: Some("specials".into()),
            movie_tmdb_id: None,
            movie_mal_id: None,
            movie_anidb_id: None,
        }],
        interstitial_season_episode: None,
        monitored: true,
        created_at: Utc::now(),
    };
    ShowRepository::create_collection(&catalog, collection.clone())
        .await
        .expect("insert collection");

    let episode = Episode {
        id: "episode-show-1".into(),
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".into()),
        season_number: Some("1".into()),
        episode_label: Some("Pilot".into()),
        title: Some("Pilot".into()),
        air_date: None,
        duration_seconds: Some(1000),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: Some("The pilot episode.".into()),
        tvdb_id: None,
        monitored: true,
        created_at: Utc::now(),
    };
    ShowRepository::create_episode(&catalog, episode.clone())
        .await
        .expect("insert episode");

    let collections = ShowRepository::list_collections_for_title(&catalog, &title.id)
        .await
        .expect("list collections");
    let episodes = ShowRepository::list_episodes_for_collection(&catalog, &collection.id)
        .await
        .expect("list episodes");

    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].id, collection.id);
    assert_eq!(
        collections[0]
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.name.as_str()),
        Some("Test Movie")
    );
    let loaded_collection = ShowRepository::get_collection_by_id(&catalog, &collection.id)
        .await
        .expect("get collection by id")
        .expect("collection should exist");
    assert_eq!(loaded_collection.id, collection.id);
    assert_eq!(
        loaded_collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.imdb_id.as_str()),
        Some("tt1234567")
    );
    assert_eq!(loaded_collection.specials_movies.len(), 1);
    assert_eq!(
        loaded_collection.specials_movies[0].movie_form.as_deref(),
        Some("recap")
    );
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].id, episode.id);
    let loaded_episode = ShowRepository::get_episode_by_id(&catalog, &episode.id)
        .await
        .expect("get episode by id")
        .expect("episode should exist");
    assert_eq!(loaded_episode.id, episode.id);

    let updated_collection = ShowRepository::update_collection(
        &catalog,
        &collection.id,
        CollectionUpdate {
            collection_type: Some(CollectionType::Arc),
            collection_index: Some("1.1".into()),
            label: Some("Arc One".into()),
            ordered_path: Some("arc/season".into()),
            last_episode_number: Some("12".into()),
            ..Default::default()
        },
    )
    .await
    .expect("update collection");
    assert_eq!(updated_collection.collection_type, CollectionType::Arc);
    assert_eq!(updated_collection.collection_index, "1.1");
    assert_eq!(updated_collection.label, Some("Arc One".into()));
    assert_eq!(updated_collection.ordered_path, Some("arc/season".into()));
    assert_eq!(updated_collection.last_episode_number, Some("12".into()));

    let updated_episode = ShowRepository::update_episode(
        &catalog,
        &episode.id,
        EpisodeUpdate {
            episode_type: Some(scryer_domain::EpisodeType::Special),
            episode_number: Some("E1".into()),
            season_number: Some("2".into()),
            episode_label: Some("Special".into()),
            title: Some("Pilot Special".into()),
            air_date: Some("2026-01-01".into()),
            duration_seconds: Some(2_400),
            has_multi_audio: Some(true),
            has_subtitle: Some(false),
            collection_id: Some(collection.id.clone()),
            overview: Some("Updated overview".into()),
            tvdb_id: Some("349232".into()),
            ..Default::default()
        },
    )
    .await
    .expect("update episode");
    assert_eq!(
        updated_episode.episode_type,
        scryer_domain::EpisodeType::Special
    );
    assert_eq!(updated_episode.episode_number, Some("E1".into()));
    assert_eq!(updated_episode.season_number, Some("2".into()));
    assert_eq!(updated_episode.episode_label, Some("Special".into()));
    assert_eq!(updated_episode.title, Some("Pilot Special".into()));
    assert_eq!(updated_episode.air_date, Some("2026-01-01".into()));
    assert_eq!(updated_episode.duration_seconds, Some(2_400));
    assert!(updated_episode.has_multi_audio);
    assert!(!updated_episode.has_subtitle);

    ShowRepository::delete_episode(&catalog, &episode.id)
        .await
        .expect("delete episode");
    let episodes_after_delete =
        ShowRepository::list_episodes_for_collection(&catalog, &collection.id)
            .await
            .expect("list episodes after delete");
    assert!(episodes_after_delete.is_empty());
    let missing_episode = ShowRepository::get_episode_by_id(&catalog, &episode.id)
        .await
        .expect("get episode by id after delete");
    assert!(missing_episode.is_none());

    ShowRepository::delete_collection(&catalog, &collection.id)
        .await
        .expect("delete collection");
    let collections_after_delete = ShowRepository::list_collections_for_title(&catalog, &title.id)
        .await
        .expect("list collections after delete");
    assert!(collections_after_delete.is_empty());
    let missing_collection = ShowRepository::get_collection_by_id(&catalog, &collection.id)
        .await
        .expect("get collection by id after delete");
    assert!(missing_collection.is_none());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn library_scan_unmatched_items_round_trip_and_preserve_created_at() {
    let db = std::env::temp_dir().join(format!(
        "scryer_scan_unmatched_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let library_state = library_state_store(&services);

    let created_at = "2026-04-07T00:00:00Z".to_string();
    let updated_at = "2026-04-07T00:00:00Z".to_string();
    let item = LibraryScanUnmatchedItem {
        id: "library_scan_unmatched:test".to_string(),
        facet: MediaFacet::Movie,
        scan_session_id: "session-1".to_string(),
        scan_root: "/library".to_string(),
        item_path: "/library/Unknown.Movie.2020.mkv".to_string(),
        display_name: "Unknown.Movie.2020".to_string(),
        query: "Unknown Movie".to_string(),
        year_hint: Some(2020),
        reason_code: "no_metadata_search_results".to_string(),
        error_message: None,
        search_attempts: vec![LibraryScanUnmatchedSearchAttempt {
            query: "Unknown Movie".to_string(),
            result_count: 0,
            top_results: Vec::new(),
        }],
        created_at: created_at.clone(),
        updated_at: updated_at.clone(),
    };

    library_state
        .upsert_library_scan_unmatched_item(&item)
        .await
        .expect("insert unmatched item");

    let count = library_state
        .count_library_scan_unmatched_items(Some(MediaFacet::Movie), Some("/library"))
        .await
        .expect("count unmatched items after insert");
    assert_eq!(count, 1);

    let listed = library_state
        .list_library_scan_unmatched_items(Some(MediaFacet::Movie), Some("/library"), 10, 0)
        .await
        .expect("list unmatched items after insert");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].search_attempts.len(), 1);
    assert_eq!(listed[0].search_attempts[0].query, "Unknown Movie");
    assert_eq!(listed[0].created_at, created_at);

    let updated = LibraryScanUnmatchedItem {
        scan_session_id: "session-2".to_string(),
        reason_code: "no_acceptable_metadata_match".to_string(),
        search_attempts: vec![LibraryScanUnmatchedSearchAttempt {
            query: "Unknown Movie 2020".to_string(),
            result_count: 2,
            top_results: vec![
                "Known Movie (2019)".to_string(),
                "Known Movie 2 (2020)".to_string(),
            ],
        }],
        created_at: "2026-04-08T00:00:00Z".to_string(),
        updated_at: "2026-04-08T01:00:00Z".to_string(),
        ..item.clone()
    };

    library_state
        .upsert_library_scan_unmatched_item(&updated)
        .await
        .expect("update unmatched item");

    let listed_after_update = library_state
        .list_library_scan_unmatched_items(Some(MediaFacet::Movie), Some("/library"), 10, 0)
        .await
        .expect("list unmatched items after update");
    assert_eq!(listed_after_update.len(), 1);
    assert_eq!(listed_after_update[0].scan_session_id, "session-2");
    assert_eq!(
        listed_after_update[0].reason_code,
        "no_acceptable_metadata_match"
    );
    assert_eq!(listed_after_update[0].created_at, item.created_at);
    assert_eq!(listed_after_update[0].updated_at, updated.updated_at);
    assert_eq!(listed_after_update[0].search_attempts[0].result_count, 2);

    library_state
        .delete_library_scan_unmatched_item(MediaFacet::Movie, &item.item_path)
        .await
        .expect("delete unmatched item");

    let count_after_delete = library_state
        .count_library_scan_unmatched_items(Some(MediaFacet::Movie), Some("/library"))
        .await
        .expect("count unmatched items after delete");
    assert_eq!(count_after_delete, 0);

    let _ = std::fs::remove_file(db);
}
