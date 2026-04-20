use super::*;
use chrono::Utc;
use scryer_application::{
    CollectionUpdate, DownloadQueueCommandRepository, DownloadSubmissionRepository, EpisodeUpdate,
    ImportRepository, InsertMediaFileInput, LibraryScanUnmatchedItem,
    LibraryScanUnmatchedItemRepository, LibraryScanUnmatchedSearchAttempt, MediaFileRepository,
    ShowRepository, TitleImageBlob, TitleImageKind, TitleImageReplacement, TitleImageRepository,
    TitleImageStorageMode, TitleImageVariantRecord, TitleMetadataUpdate, TitleRepository,
    UserRepository, WantedItemRepository,
};
use scryer_domain::{
    Collection, CollectionType, Entitlement, Episode, ExternalId, ImportType,
    InterstitialMovieMetadata, MediaFacet, Title,
};
use sqlx::{Row, sqlite::SqlitePoolOptions};
use tokio::time::{Duration, timeout};

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

#[tokio::test]
async fn list_imports_for_sources_handles_multiple_pairs() {
    let db = std::env::temp_dir().join(format!(
        "scryer_import_sources_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = SqliteWorkflowStore::new(&services);

    workflow
        .queue_import_request(
            "weaver".to_string(),
            "10000".to_string(),
            ImportType::ManualImport.as_str().to_string(),
            "{}".to_string(),
        )
        .await
        .expect("first import should queue");
    workflow
        .queue_import_request(
            "weaver".to_string(),
            "10001".to_string(),
            ImportType::ManualImport.as_str().to_string(),
            "{}".to_string(),
        )
        .await
        .expect("second import should queue");

    let records = workflow
        .list_imports_for_sources(&[
            ("weaver".to_string(), "10000".to_string()),
            ("weaver".to_string(), "10001".to_string()),
        ])
        .await
        .expect("batch lookup should succeed");

    assert_eq!(records.len(), 2);

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

async fn run_embedded_migration(pool: &sqlx::SqlitePool, sql: &str) {
    for statement in sql
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
    {
        sqlx::query(statement)
            .execute(pool)
            .await
            .expect("migration statement should succeed");
    }
}

async fn create_pre_0079_title_projection_schema(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "CREATE TABLE titles (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            facet TEXT NOT NULL,
            external_ids TEXT NOT NULL DEFAULT '[]',
            metadata_fetched_at TEXT
        )",
    )
    .execute(pool)
    .await
    .expect("create legacy titles");

    sqlx::query(
        "CREATE TABLE title_external_ids (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            source TEXT NOT NULL,
            external_id TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .expect("create legacy title_external_ids");

    sqlx::query(
        "CREATE UNIQUE INDEX idx_title_external_ids_lookup
         ON title_external_ids(source, external_id)",
    )
    .execute(pool)
    .await
    .expect("create legacy title_external_ids lookup");
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
    assert_eq!(
        after_cache.poster_source_url.as_deref(),
        Some("https://tvdb.example/poster.jpg")
    );

    let listed = TitleRepository::list(&catalog, None, None)
        .await
        .expect("title list should succeed");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].poster_url.as_deref(),
        Some("/images/titles/title-1/poster/w500?v=bbbbbbbbbbbbbbbb")
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn hydrated_title_metadata_with_extra_external_ids_completes_on_single_connection_sqlite() {
    let services = SqliteServices::new("sqlite://file::memory:?mode=memory&cache=shared")
        .await
        .expect("in-memory db should initialize");
    let catalog = catalog_store(&services);

    let mut title = make_test_title("title-hydration-extra-ids", None);
    title.facet = MediaFacet::Anime;
    title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "12345".to_string(),
    }];

    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let update = TitleMetadataUpdate {
        metadata_language: Some("eng".to_string()),
        metadata_fetched_at: Some(Utc::now().to_rfc3339()),
        extra_external_ids: vec![
            ExternalId {
                source: "mal".to_string(),
                value: "834".to_string(),
            },
            ExternalId {
                source: "anilist".to_string(),
                value: "269".to_string(),
            },
        ],
        ..TitleMetadataUpdate::default()
    };

    let updated = timeout(
        Duration::from_secs(1),
        TitleRepository::update_title_hydrated_metadata(&catalog, &title.id, update),
    )
    .await
    .expect("hydrated metadata update should not self-deadlock on single-connection sqlite")
    .expect("hydrated metadata update should succeed");

    assert!(
        updated
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "mal" && external_id.value == "834" })
    );
    assert!(
        updated
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "anilist" && external_id.value == "269" })
    );
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
    assert_eq!(
        updated.poster_source_url.as_deref(),
        Some("https://tvdb.example/poster-b.jpg")
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn create_title_only_marks_tvdb_titles_for_background_hydration() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_hydration_seed_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = catalog_store(&services);

    let mut tvdb_title = make_test_title("title-tvdb", None);
    tvdb_title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "123".to_string(),
    }];
    TitleRepository::create(&catalog, tvdb_title)
        .await
        .expect("tvdb title should insert");

    let mut imdb_title = make_test_title("title-imdb", None);
    imdb_title.external_ids = vec![ExternalId {
        source: "imdb".to_string(),
        value: "tt1234567".to_string(),
    }];
    TitleRepository::create(&catalog, imdb_title)
        .await
        .expect("imdb title should insert");

    let markers: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, metadata_hydration_next_attempt_at
         FROM titles
         WHERE id IN (?, ?)
         ORDER BY id",
    )
    .bind("title-imdb")
    .bind("title-tvdb")
    .fetch_all(&services.pool)
    .await
    .expect("load hydration markers");

    assert_eq!(markers[0], ("title-imdb".to_string(), None));
    assert!(
        markers[1].1.is_some(),
        "tvdb-backed titles should be queued for background hydration"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn list_titles_due_for_hydration_excludes_active_facets_in_due_order() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_hydration_excluded_facets_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = catalog_store(&services);

    let mut anime_title = make_test_title("anime-due", None);
    anime_title.facet = MediaFacet::Anime;
    anime_title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "301".to_string(),
    }];
    TitleRepository::create(&catalog, anime_title)
        .await
        .expect("anime title should insert");

    let mut movie_title = make_test_title("movie-due", None);
    movie_title.facet = MediaFacet::Movie;
    movie_title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "101".to_string(),
    }];
    TitleRepository::create(&catalog, movie_title)
        .await
        .expect("movie title should insert");

    let mut series_title = make_test_title("series-due", None);
    series_title.facet = MediaFacet::Series;
    series_title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "201".to_string(),
    }];
    TitleRepository::create(&catalog, series_title)
        .await
        .expect("series title should insert");

    sqlx::query(
        "UPDATE titles
         SET metadata_hydration_next_attempt_at = ?,
             metadata_hydration_attempt_count = 0
         WHERE id IN (?, ?, ?)",
    )
    .bind("2026-01-01T00:00:00Z")
    .bind("anime-due")
    .bind("movie-due")
    .bind("series-due")
    .execute(&services.pool)
    .await
    .expect("normalize due timestamps");

    let due_titles =
        TitleRepository::list_titles_due_for_hydration(&catalog, 10, &[MediaFacet::Series])
            .await
            .expect("load due titles excluding active series facet");

    let due_ids = due_titles
        .into_iter()
        .map(|pending| pending.title.id)
        .collect::<Vec<_>>();
    assert_eq!(
        due_ids,
        vec!["anime-due".to_string(), "movie-due".to_string()]
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
    let library_state = library_state_store(&services);

    let mut title = make_test_title(
        "title-external-id",
        Some("https://tvdb.example/poster-external.jpg"),
    );
    title.external_ids = vec![ExternalId {
        source: "TVDB".to_string(),
        value: "123456".to_string(),
    }];
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    library_state
        .replace_title_image(
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Poster,
                source_url: "https://tvdb.example/poster-external.jpg".to_string(),
                source_etag: None,
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 1000,
                source_height: 1500,
                storage_mode: TitleImageStorageMode::AvifMaster,
                master_format: "avif".to_string(),
                master_sha256: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string(),
                master_width: 1000,
                master_height: 1500,
                master_bytes: vec![1, 1, 1],
                variants: vec![TitleImageVariantRecord {
                    variant_key: "w500".to_string(),
                    format: "avif".to_string(),
                    width: 500,
                    height: 750,
                    bytes: vec![2, 2, 2],
                    sha256: "ffffffffffffffffffffffffffffffff".to_string(),
                }],
            },
        )
        .await
        .expect("title image should insert");

    let found = catalog
        .find_by_external_id("tvdb", "123456")
        .await
        .expect("lookup should succeed")
        .expect("title should exist");

    assert_eq!(found.id, title.id);
    assert_eq!(
        found.poster_url.as_deref(),
        Some("/images/titles/title-external-id/poster/w500?v=ffffffffffffffff")
    );
    assert_eq!(
        found.poster_source_url.as_deref(),
        Some("https://tvdb.example/poster-external.jpg")
    );
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_list_by_external_ids_returns_unique_first_matches() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_external_id_batch_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = catalog_store(&services);

    let mut first = make_test_title("title-a", Some("https://tvdb.example/a.jpg"));
    first.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "123456".to_string(),
    }];
    TitleRepository::create(&catalog, first.clone())
        .await
        .expect("first title should insert");

    let mut duplicate = make_test_title("title-z", Some("https://tvdb.example/z.jpg"));
    duplicate.facet = MediaFacet::Series;
    duplicate.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "123456".to_string(),
    }];
    TitleRepository::create(&catalog, duplicate)
        .await
        .expect("duplicate title should insert");

    let mut second = make_test_title("title-b", Some("https://tvdb.example/b.jpg"));
    second.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "345678".to_string(),
    }];
    TitleRepository::create(&catalog, second.clone())
        .await
        .expect("second title should insert");

    let values = vec![
        "123456".to_string(),
        "123456".to_string(),
        "000000".to_string(),
        "345678".to_string(),
    ];
    let matches = catalog
        .list_by_external_ids("tvdb", &values)
        .await
        .expect("batch lookup should succeed");

    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].id, first.id);
    assert_eq!(matches[1].id, second.id);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_list_for_matching_keeps_source_image_urls() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_list_for_matching_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = catalog_store(&services);
    let library_state = library_state_store(&services);

    let title = make_test_title(
        "title-list-matching",
        Some("https://tvdb.example/poster.jpg"),
    );
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    library_state
        .replace_title_image(
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Poster,
                source_url: "https://tvdb.example/poster.jpg".to_string(),
                source_etag: None,
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 1000,
                source_height: 1500,
                storage_mode: TitleImageStorageMode::AvifMaster,
                master_format: "avif".to_string(),
                master_sha256: "abababababababababababababababab".to_string(),
                master_width: 1000,
                master_height: 1500,
                master_bytes: vec![1, 2, 3],
                variants: vec![TitleImageVariantRecord {
                    variant_key: "w500".to_string(),
                    format: "avif".to_string(),
                    width: 500,
                    height: 750,
                    bytes: vec![4, 5, 6],
                    sha256: "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd".to_string(),
                }],
            },
        )
        .await
        .expect("title image should insert");

    let titles = TitleRepository::list_for_matching(&catalog, None, None)
        .await
        .expect("matching list should succeed");
    let listed = titles
        .into_iter()
        .find(|candidate| candidate.id == title.id)
        .expect("title should be listed");

    assert_eq!(
        listed.poster_url.as_deref(),
        Some("https://tvdb.example/poster.jpg")
    );
    assert!(listed.poster_source_url.is_none());

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
async fn title_image_local_path_backfill_matches_legacy_served_path() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_backfill_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = catalog_store(&services);

    let title = make_test_title(
        "title-backfill",
        Some("https://tvdb.example/poster-backfill.jpg"),
    );
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    sqlx::query(
        "INSERT INTO title_images (
            id, title_id, provider, provider_image_id, kind, source_url, source_etag,
            source_last_modified, source_format, source_width, source_height, storage_mode,
            master_path, master_format, master_sha256, master_width, master_height, bytes,
            created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("img-backfill")
    .bind(&title.id)
    .bind("tvdb")
    .bind(Option::<String>::None)
    .bind("poster")
    .bind("https://tvdb.example/poster-backfill.jpg")
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind("jpeg")
    .bind(1000i32)
    .bind(1500i32)
    .bind("avif_master")
    .bind(Option::<String>::None)
    .bind("avif")
    .bind("12121212121212121212121212121212")
    .bind(1000i32)
    .bind(1500i32)
    .bind(vec![1_u8, 2, 3])
    .bind(Utc::now().to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(&services.pool)
    .await
    .expect("legacy title image row should insert");

    sqlx::query(
        "INSERT INTO title_image_variants (
            id, title_image_id, variant_key, path, format, width, height, bytes, sha256, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("variant-backfill")
    .bind("img-backfill")
    .bind("w500")
    .bind(Option::<String>::None)
    .bind("avif")
    .bind(500i32)
    .bind(750i32)
    .bind(vec![4_u8, 5, 6])
    .bind("34343434343434343434343434343434")
    .bind(Utc::now().to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(&services.pool)
    .await
    .expect("legacy title image variant row should insert");

    sqlx::query("UPDATE titles SET poster_local_path = NULL WHERE id = ?")
        .bind(&title.id)
        .execute(&services.pool)
        .await
        .expect("local path should be cleared to simulate legacy state");

    for statement in
        include_str!("../../scryer/src/db/migrations/0075_title_image_local_paths.sql").split(";\n")
    {
        let statement = statement.trim();
        if statement.is_empty() {
            continue;
        }
        if statement.starts_with("ALTER TABLE titles ADD COLUMN") {
            let _ = sqlx::query(statement).execute(&services.pool).await;
            continue;
        }
        sqlx::query(statement)
            .execute(&services.pool)
            .await
            .expect("backfill statement should succeed");
    }

    let materialized_path: Option<String> =
        sqlx::query_scalar("SELECT poster_local_path FROM titles WHERE id = ?")
            .bind(&title.id)
            .fetch_one(&services.pool)
            .await
            .expect("materialized local path should exist");
    assert_eq!(
        materialized_path.as_deref(),
        Some("/images/titles/title-backfill/poster/w500?v=3434343434343434")
    );

    let hydrated = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        hydrated.poster_url.as_deref(),
        Some("/images/titles/title-backfill/poster/w500?v=3434343434343434")
    );
    assert_eq!(
        hydrated.poster_source_url.as_deref(),
        Some("https://tvdb.example/poster-backfill.jpg")
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

#[tokio::test]
async fn complete_wanted_item_for_title_updates_matching_row_in_one_step() {
    let db = std::env::temp_dir().join(format!(
        "scryer_complete_wanted_item_for_title_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = library_state_store(&services);
    let catalog = catalog_store(&services);
    let now = Utc::now().to_rfc3339();

    let title = make_test_title("title-series", None);
    TitleRepository::create(&catalog, title)
        .await
        .expect("title should insert");

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, media_type, search_phase, status, search_count,
          current_score, grabbed_release, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("wanted-episode")
    .bind("title-series")
    .bind("movie")
    .bind("primary")
    .bind("wanted")
    .bind(7i64)
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
        "SELECT status, next_search_at, last_search_at, search_count, current_score, grabbed_release
         FROM wanted_items
         WHERE id = ?",
    )
    .bind("wanted-episode")
    .fetch_one(services.pool())
    .await
    .expect("wanted item should load");

    assert_eq!(row.get::<String, _>("status"), "completed");
    assert_eq!(row.get::<Option<String>, _>("next_search_at"), None);
    assert_eq!(
        row.get::<Option<String>, _>("last_search_at"),
        Some("2026-04-20T00:00:00Z".to_string())
    );
    assert_eq!(row.get::<i64, _>("search_count"), 7);
    assert_eq!(row.get::<Option<i64>, _>("current_score"), Some(42));
    assert_eq!(
        row.get::<Option<String>, _>("grabbed_release"),
        Some("Existing Release".to_string())
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0079_faceted_projection_allows_cross_facet_duplicates_and_seeds_only_tvdb_titles()
 {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_facets_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL)",
    )
    .bind("series-1")
    .bind("Series")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"123"}]"#)
    .bind("movie-1")
    .bind("Movie")
    .bind("movie")
    .bind(r#"[{"source":"tvdb","value":"123"}]"#)
    .bind("movie-imdb")
    .bind("IMDb Only")
    .bind("movie")
    .bind(r#"[{"source":"imdb","value":"tt1234567"}]"#)
    .execute(&pool)
    .await
    .expect("insert legacy titles");

    run_embedded_migration(
        &pool,
        include_str!("../../scryer/src/db/migrations/0079_title_external_id_projection_and_metadata_hydration_retry.sql"),
    )
    .await;

    let faceted_rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT title_id, facet, external_id
         FROM title_external_ids
         WHERE source = 'tvdb'
         ORDER BY facet, title_id",
    )
    .fetch_all(&pool)
    .await
    .expect("load projected faceted tvdb ids");
    assert_eq!(
        faceted_rows,
        vec![
            (
                "movie-1".to_string(),
                "movie".to_string(),
                "123".to_string()
            ),
            (
                "series-1".to_string(),
                "series".to_string(),
                "123".to_string()
            ),
        ]
    );

    let due_now: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, metadata_hydration_next_attempt_at
         FROM titles
         ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("load hydration due markers");
    assert!(
        due_now
            .iter()
            .find(|(id, _)| id == "movie-imdb")
            .expect("imdb title marker")
            .1
            .is_none()
    );
    assert!(
        due_now
            .iter()
            .find(|(id, _)| id == "movie-1")
            .expect("movie tvdb marker")
            .1
            .is_some()
    );
    assert!(
        due_now
            .iter()
            .find(|(id, _)| id == "series-1")
            .expect("series tvdb marker")
            .1
            .is_some()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0079_rejects_same_facet_duplicate_before_delete() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_duplicate_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO title_external_ids
         (id, title_id, source, external_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("legacy-row")
    .bind("legacy-title")
    .bind("tvdb")
    .bind("legacy")
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy projection row");

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL)",
    )
    .bind("series-a")
    .bind("Series A")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .bind("series-b")
    .bind("Series B")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .execute(&pool)
    .await
    .expect("insert conflicting legacy titles");

    let migration_sql = include_str!(
        "../../scryer/src/db/migrations/0079_title_external_id_projection_and_metadata_hydration_retry.sql"
    );
    let err = {
        let mut failed = None;
        for statement in migration_sql
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            if let Err(error) = sqlx::query(statement).execute(&pool).await {
                failed = Some(error);
                break;
            }
        }
        failed.expect("migration should fail on same-facet duplicate")
    };
    assert!(
        err.to_string().contains("UNIQUE"),
        "expected uniqueness failure, got: {err}"
    );

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_external_ids")
        .fetch_one(&pool)
        .await
        .expect("load remaining legacy projection rows");
    assert_eq!(remaining, 1);

    let legacy_external_id: String =
        sqlx::query_scalar("SELECT external_id FROM title_external_ids WHERE id = 'legacy-row'")
            .fetch_one(&pool)
            .await
            .expect("legacy row should remain");
    assert_eq!(legacy_external_id, "legacy");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0079_conflict_hint_lists_colliding_title_ids() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_conflict_hint_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL)",
    )
    .bind("series-a")
    .bind("Series A")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .bind("series-b")
    .bind("Series B")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .execute(&pool)
    .await
    .expect("insert conflicting legacy titles");

    let hint = crate::migrations::title_external_id_projection_conflict_hint(&pool)
        .await
        .expect("conflict hint should be present");
    assert!(hint.contains("series/tvdb/999"));
    assert!(hint.contains("series-a"));
    assert!(hint.contains("series-b"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0079_rejects_invalid_projection_before_delete() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_invalid_json_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO title_external_ids
         (id, title_id, source, external_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("legacy-row")
    .bind("legacy-title")
    .bind("tvdb")
    .bind("legacy")
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy projection row");

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL)",
    )
    .bind("series-bad")
    .bind("Broken Series")
    .bind("series")
    .bind("{not-valid-json")
    .execute(&pool)
    .await
    .expect("insert malformed legacy title");

    let migration_sql = include_str!(
        "../../scryer/src/db/migrations/0079_title_external_id_projection_and_metadata_hydration_retry.sql"
    );
    let err = {
        let mut failed = None;
        for statement in migration_sql
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            if let Err(error) = sqlx::query(statement).execute(&pool).await {
                failed = Some(error);
                break;
            }
        }
        failed.expect("migration should fail on malformed external_ids json")
    };
    assert!(
        err.to_string().contains("malformed"),
        "expected malformed json failure, got: {err}"
    );

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_external_ids")
        .fetch_one(&pool)
        .await
        .expect("load remaining legacy projection rows");
    assert_eq!(remaining, 1);

    let _ = std::fs::remove_file(db);
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
async fn queued_delete_stale_recovery_only_recovers_stale_rows() {
    let db = std::env::temp_dir().join(format!(
        "scryer_delete_recovery_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow_store = SqliteWorkflowStore::new(&services);

    let stale = workflow_store
        .queue_delete_command("nzbget", "job-stale", false, Some("admin"))
        .await
        .expect("stale delete should queue");
    let fresh = workflow_store
        .queue_delete_command("nzbget", "job-fresh", true, Some("admin"))
        .await
        .expect("fresh delete should queue");

    workflow_store
        .mark_delete_command_running(&stale.id)
        .await
        .expect("stale delete should mark running");
    workflow_store
        .mark_delete_command_running(&fresh.id)
        .await
        .expect("fresh delete should mark running");

    let stale_updated_at = (Utc::now() - chrono::Duration::seconds(300)).to_rfc3339();
    sqlx::query("UPDATE download_queue_commands SET updated_at = ? WHERE id = ?")
        .bind(&stale_updated_at)
        .bind(&stale.id)
        .execute(&services.pool)
        .await
        .expect("age stale running delete");

    let recovered = workflow_store
        .recover_stale_running_delete_commands(120)
        .await
        .expect("stale recovery should succeed");
    assert_eq!(recovered, 1);

    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, status, started_at
         FROM download_queue_commands
         WHERE id IN (?, ?)
         ORDER BY id",
    )
    .bind(&fresh.id)
    .bind(&stale.id)
    .fetch_all(&services.pool)
    .await
    .expect("load delete rows after stale recovery");

    assert_eq!(rows.len(), 2);
    let fresh_row = rows
        .iter()
        .find(|row| row.0 == fresh.id)
        .expect("fresh row should exist");
    assert_eq!(fresh_row.1, "running");
    assert!(
        fresh_row.2.is_some(),
        "fresh running delete should remain running"
    );
    let stale_row = rows
        .iter()
        .find(|row| row.0 == stale.id)
        .expect("stale row should exist");
    assert_eq!(stale_row, &(stale.id, "queued".to_string(), None));

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
