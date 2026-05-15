use std::sync::{Arc, RwLock};

use scryer_application::{AppError, AppResult};
use sqlx::ConnectOptions;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tracing::log::LevelFilter;

use crate::encryption::EncryptionKey;
use crate::types::MigrationMode;

const DEFAULT_POSTGRES_MAX_CONNECTIONS: u32 = 16;
const MAX_POSTGRES_CONNECTIONS_CAP: u32 = 128;
const POSTGRES_SLOW_STATEMENT_WARN_MS: u64 = 1000;

fn postgres_max_connections_from_env() -> u32 {
    std::env::var("SCRYER_POSTGRES_MAX_CONNECTIONS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_POSTGRES_MAX_CONNECTIONS)
        .clamp(1, MAX_POSTGRES_CONNECTIONS_CAP)
}

#[derive(Clone)]
pub struct PostgresServices {
    pool: sqlx::PgPool,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl PostgresServices {
    pub async fn new_with_mode(
        database_url: impl AsRef<str>,
        migration_mode: MigrationMode,
    ) -> Result<Self, AppError> {
        let mut connect_options: PgConnectOptions =
            database_url
                .as_ref()
                .parse()
                .map_err(|error: sqlx::Error| {
                    AppError::Repository(format!("invalid PostgreSQL database URL: {error}"))
                })?;
        connect_options = connect_options.log_slow_statements(
            LevelFilter::Warn,
            std::time::Duration::from_millis(POSTGRES_SLOW_STATEMENT_WARN_MS),
        );

        let pool = PgPoolOptions::new()
            .max_connections(postgres_max_connections_from_env())
            .connect_with(connect_options)
            .await
            .map_err(|error| {
                AppError::Repository(format!("cannot open PostgreSQL database: {error}"))
            })?;

        super::migrations::run_migrations(&pool, migration_mode).await?;

        Ok(Self {
            pool,
            encryption_key: Arc::new(RwLock::new(None)),
        })
    }

    pub fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }

    pub(crate) fn encryption_key_state(&self) -> Arc<RwLock<Option<EncryptionKey>>> {
        self.encryption_key.clone()
    }

    pub async fn set_encryption_key(&self, key: EncryptionKey) -> AppResult<()> {
        *self
            .encryption_key
            .write()
            .map_err(|_| AppError::Repository("encryption key lock poisoned".to_string()))? =
            Some(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    use super::*;
    use scryer_application::{
        LibraryRepository, LibraryScanUnmatchedItem, LibraryScanUnmatchedItemRepository,
        LibraryScanUnmatchedSearchAttempt, PendingImportStatus, QualityProfileRepository,
        SettingsRepository, ShowRepository, SystemInfoProvider, TitleImageKind,
        TitleImageReplacement, TitleImageRepository, TitleImageStorageMode,
        TitleImageVariantRecord, TitleMetadataUpdate, TitleRepository, UserRepository,
        default_quality_profile_for_search,
    };
    use scryer_domain::{
        Collection, CollectionType, ExternalId, Id, InterstitialMovieMetadata, LibraryGrant,
        LibraryPermission, LibraryPermissionMask, MediaFacet, Title, User,
    };
    use sqlx::Row;
    use tokio::task::JoinSet;

    use crate::SettingDefinitionSeed;

    #[tokio::test]
    async fn postgres_blank_install_smoke_from_env_url() -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!("skipping PostgreSQL smoke test; SCRYER_TEST_POSTGRES_URL is not set");
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let catalog = super::super::PostgresCatalogStore::new(&services);
            let images = super::super::PostgresLibraryStateStore::new(&services);
            let settings = super::super::PostgresSettingsStore::new(&services);
            let info = settings.datastore_info().await?;
            assert_eq!(info.engine, "postgres");
            let current_migration_key = info.current_migration_key.as_deref().ok_or_else(|| {
                AppError::Repository(
                    "expected PostgreSQL blank install to record a current migration key"
                        .to_string(),
                )
            })?;
            let current_migration_version = current_migration_key
                .split('_')
                .next()
                .and_then(|value| value.parse::<u32>().ok())
                .ok_or_else(|| {
                    AppError::Repository(format!(
                        "failed to parse PostgreSQL migration key {current_migration_key}"
                    ))
                })?;
            assert!(
                current_migration_version >= 111,
                "expected PostgreSQL blank install to apply through 0111+, got {current_migration_key}",
            );
            assert_postgres_runtime_schema_columns(services.pool()).await?;
            assert_schema_parity_owned_tables_match_sqlite(services.pool()).await?;

            let title = sample_title("pg-blank-install-movie");
            TitleRepository::create(&catalog, title.clone()).await?;
            let title_search_term_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM title_search_terms WHERE title_id = $1")
                    .bind(&title.id)
                    .fetch_one(services.pool())
                    .await
                    .map_err(|error| AppError::Repository(error.to_string()))?;
            assert!(
                title_search_term_count > 0,
                "PostgreSQL title create should seed canonical title_search_terms"
            );
            catalog
                .mark_title_metadata_hydration_due_now(&title.id)
                .await?;

            let due = catalog.list_titles_due_for_hydration(10, &[]).await?;
            assert_eq!(due.len(), 1, "expected one title due for hydration");
            assert_eq!(due[0].title.id, title.id);

            catalog
                .update_title_hydrated_metadata(
                    &title.id,
                    TitleMetadataUpdate {
                        name: None,
                        year: Some(2024),
                        overview: Some("postgres blank install smoke".to_string()),
                        poster_url: Some("https://example.com/poster.jpg".to_string()),
                        banner_url: Some("https://example.com/banner.jpg".to_string()),
                        background_url: Some("https://example.com/fanart.jpg".to_string()),
                        sort_title: Some("Postgres Blank Install Smoke".to_string()),
                        slug: Some("postgres-blank-install-smoke".to_string()),
                        imdb_id: Some("tt1234567".to_string()),
                        runtime_minutes: Some(123),
                        genres: vec!["Drama".to_string()],
                        content_status: Some("released".to_string()),
                        language: Some("eng".to_string()),
                        first_aired: Some("2024-01-01".to_string()),
                        network: Some("Example Network".to_string()),
                        studio: Some("Example Studio".to_string()),
                        country: Some("US".to_string()),
                        aliases: vec!["Smoke Movie".to_string()],
                        tagged_aliases: Vec::new(),
                        metadata_language: Some("eng".to_string()),
                        metadata_fetched_at: Some(chrono::Utc::now().to_rfc3339()),
                        digital_release_date: Some("2024-01-15".to_string()),
                        extra_external_ids: Vec::new(),
                        extra_tags: Vec::new(),
                    },
                )
                .await?;
            catalog
                .clear_title_metadata_hydration_retry_state(&title.id)
                .await?;

            let poster_tasks = images
                .list_titles_requiring_image_refresh(TitleImageKind::Poster, 10)
                .await?;
            assert_eq!(
                poster_tasks.len(),
                1,
                "expected hydrated poster metadata to drive image refresh work"
            );
            assert_eq!(poster_tasks[0].title_id, title.id);
            assert_eq!(
                poster_tasks[0].source_url,
                "https://example.com/poster.jpg".to_string()
            );

            images
                .replace_title_image(
                    &title.id,
                    TitleImageReplacement {
                        kind: TitleImageKind::Poster,
                        source_url: "https://example.com/poster.jpg".to_string(),
                        source_etag: Some("source-etag".to_string()),
                        source_last_modified: Some("Wed, 14 May 2026 03:00:00 GMT".to_string()),
                        source_format: "jpeg".to_string(),
                        source_width: 1200,
                        source_height: 1800,
                        storage_mode: TitleImageStorageMode::Original,
                        master_format: "jpeg".to_string(),
                        master_sha256: "poster-master-sha256".to_string(),
                        master_width: 1200,
                        master_height: 1800,
                        master_bytes: vec![1, 2, 3, 4],
                        variants: vec![TitleImageVariantRecord {
                            variant_key: "thumb".to_string(),
                            format: "avif".to_string(),
                            width: 240,
                            height: 360,
                            bytes: vec![5, 6, 7, 8],
                            sha256: "poster-thumb-sha256".to_string(),
                        }],
                    },
                )
                .await?;

            let original_blob = images
                .get_title_image_blob(&title.id, TitleImageKind::Poster, "original")
                .await?
                .ok_or_else(|| {
                    AppError::Repository(
                        "expected PostgreSQL blank install to persist title image master blob"
                            .to_string(),
                    )
                })?;
            assert_eq!(original_blob.content_type, "image/jpeg");
            assert_eq!(original_blob.etag, "poster-master-sha256");
            assert_eq!(original_blob.bytes, vec![1, 2, 3, 4]);

            let thumb_blob = images
                .get_title_image_blob(&title.id, TitleImageKind::Poster, "thumb")
                .await?
                .ok_or_else(|| {
                    AppError::Repository(
                        "expected PostgreSQL blank install to persist title image variants"
                            .to_string(),
                    )
                })?;
            assert_eq!(thumb_blob.content_type, "image/avif");
            assert_eq!(thumb_blob.etag, "poster-thumb-sha256");
            assert_eq!(thumb_blob.bytes, vec![5, 6, 7, 8]);

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_settings_accept_raw_strings_and_read_them_back_as_json_strings()
    -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL raw-string settings test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let settings = super::super::PostgresSettingsStore::new(&services);

            settings
                .batch_ensure_setting_definitions(vec![SettingDefinitionSeed {
                    category: "tests".to_string(),
                    scope: "tests".to_string(),
                    key_name: "raw_string_setting".to_string(),
                    data_type: "string".to_string(),
                    default_value_json: "postgres-default".to_string(),
                    is_sensitive: false,
                    validation_json: None,
                }])
                .await?;

            assert_eq!(
                SettingsRepository::get_setting_json(
                    &settings,
                    "tests",
                    "raw_string_setting",
                    None
                )
                .await?,
                Some("\"postgres-default\"".to_string()),
                "default string settings should remain valid JSON strings for application readers",
            );

            SettingsRepository::upsert_setting_json(
                &settings,
                "tests",
                "raw_string_setting",
                None,
                "preferred-profile".to_string(),
                "test",
                None,
            )
            .await?;

            assert_eq!(
                SettingsRepository::get_setting_json(
                    &settings,
                    "tests",
                    "raw_string_setting",
                    None
                )
                .await?,
                Some("\"preferred-profile\"".to_string()),
                "raw string writes should round-trip as JSON strings so application readers stay engine-neutral",
            );
            assert_eq!(
                SettingsRepository::get_setting_json_explicit(
                    &settings,
                    "tests",
                    "raw_string_setting",
                    None,
                )
                .await?,
                Some("\"preferred-profile\"".to_string()),
                "explicit raw string writes should round-trip as JSON strings so application readers stay engine-neutral",
            );

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_collection_round_trips_canonical_interstitial_columns() -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL collection interstitial-column test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let catalog = super::super::PostgresCatalogStore::new(&services);

            let mut title = sample_title("pg-collection-interstitial");
            title.facet = MediaFacet::Anime;
            TitleRepository::create(&catalog, title.clone()).await?;

            let interstitial_movie = InterstitialMovieMetadata {
                tvdb_id: "movie-100".to_string(),
                name: "Interstitial Movie".to_string(),
                slug: "interstitial-movie".to_string(),
                year: Some(2024),
                content_status: "released".to_string(),
                overview: "A canonical scalar interstitial movie".to_string(),
                poster_url: "https://example.com/interstitial.jpg".to_string(),
                language: "eng".to_string(),
                runtime_minutes: 42,
                sort_title: "Interstitial Movie".to_string(),
                imdb_id: "tt0000100".to_string(),
                genres: vec!["Animation".to_string(), "Short".to_string()],
                studio: "Scryer Studios".to_string(),
                digital_release_date: Some("2024-05-01".to_string()),
                association_confidence: Some("high".to_string()),
                continuity_status: Some("canon".to_string()),
                movie_form: Some("special".to_string()),
                confidence: Some("strong".to_string()),
                signal_summary: Some("test fixture".to_string()),
                placement: Some("between-episodes".to_string()),
                movie_tmdb_id: Some("100".to_string()),
                movie_mal_id: Some("200".to_string()),
                movie_anidb_id: Some("300".to_string()),
            };
            let special_movie = InterstitialMovieMetadata {
                tvdb_id: "movie-101".to_string(),
                name: "Special Movie".to_string(),
                slug: "special-movie".to_string(),
                year: Some(2025),
                content_status: "released".to_string(),
                overview: "A canonical special movie".to_string(),
                poster_url: "https://example.com/special.jpg".to_string(),
                language: "jpn".to_string(),
                runtime_minutes: 55,
                sort_title: "Special Movie".to_string(),
                imdb_id: "tt0000101".to_string(),
                genres: vec!["Adventure".to_string()],
                studio: "Scryer Studios".to_string(),
                digital_release_date: None,
                association_confidence: None,
                continuity_status: None,
                movie_form: Some("ova".to_string()),
                confidence: None,
                signal_summary: None,
                placement: None,
                movie_tmdb_id: None,
                movie_mal_id: Some("201".to_string()),
                movie_anidb_id: Some("301".to_string()),
            };
            let collection = Collection {
                id: "pg-collection-interstitial-season-1".to_string(),
                title_id: title.id.clone(),
                collection_type: CollectionType::Season,
                collection_index: "1".to_string(),
                label: Some("Season 1".to_string()),
                ordered_path: None,
                narrative_order: Some("1".to_string()),
                first_episode_number: Some("1".to_string()),
                last_episode_number: Some("12".to_string()),
                interstitial_movie: Some(interstitial_movie.clone()),
                specials_movies: vec![special_movie.clone()],
                interstitial_season_episode: Some("S01E06".to_string()),
                monitored: true,
                created_at: chrono::Utc::now(),
            };
            ShowRepository::create_collection(&catalog, collection.clone()).await?;

            let listed = ShowRepository::list_collections_for_title(&catalog, &title.id).await?;
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].id, collection.id);
            assert_eq!(
                listed[0].interstitial_movie,
                Some(interstitial_movie.clone())
            );
            assert_eq!(listed[0].specials_movies, vec![special_movie.clone()]);

            let loaded = ShowRepository::get_collection_by_id(&catalog, &collection.id)
                .await?
                .ok_or_else(|| {
                    AppError::Repository("expected collection to be readable by id".to_string())
                })?;
            assert_eq!(loaded.interstitial_movie, Some(interstitial_movie));
            assert_eq!(loaded.specials_movies, vec![special_movie]);
            assert_eq!(
                loaded.interstitial_season_episode.as_deref(),
                Some("S01E06")
            );

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_quality_profile_catalog_upsert_round_trips_on_blank_install() -> AppResult<()>
    {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL quality-profile catalog test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let settings = super::super::PostgresSettingsStore::new(&services);
            let profiles = vec![default_quality_profile_for_search()];

            <super::super::PostgresSettingsStore as QualityProfileRepository>::replace_quality_profiles(
                &settings,
                "system",
                None,
                profiles.clone(),
            )
            .await?;

            let stored =
                <super::super::PostgresSettingsStore as QualityProfileRepository>::list_quality_profiles(
                    &settings,
                    "system",
                    None,
                )
                .await?;
            assert_eq!(stored, profiles);

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_blank_install_applies_parity_secondary_indexes() -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!("skipping PostgreSQL parity-index test; SCRYER_TEST_POSTGRES_URL is not set");
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;

            let actual_indexes = sqlx::query_scalar::<_, String>(
                "SELECT indexname
                   FROM pg_indexes
                  WHERE schemaname = current_schema()",
            )
            .fetch_all(services.pool())
            .await
            .map(BTreeSet::from_iter)
            .map_err(|error| AppError::Repository(error.to_string()))?;

            let expected_indexes = postgres_parity_index_names_from_source();
            let missing: Vec<String> = expected_indexes
                .difference(&actual_indexes)
                .cloned()
                .collect();
            assert!(
                missing.is_empty(),
                "expected PostgreSQL blank install to apply parity secondary indexes; missing {missing:?}"
            );
            assert!(
                actual_indexes.contains("idx_library_scan_unmatched_items_facet_title_status_updated"),
                "expected PostgreSQL blank install to restore the title-aware unmatched-items index"
            );

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_library_scan_unmatched_items_round_trip_title_binding() -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL unmatched-item title-binding test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let store = super::super::PostgresLibraryStateStore::new(&services);
            let now = chrono::Utc::now().to_rfc3339();
            let item = LibraryScanUnmatchedItem {
                id: Id::new().0,
                library_id: "movie_default_library".to_string(),
                facet: MediaFacet::Movie,
                status: PendingImportStatus::Pending,
                title_id: Some("pg-title-binding".to_string()),
                scan_session_id: Id::new().0,
                scan_root: "/tmp/scryer-test/movies".to_string(),
                item_path: "/tmp/scryer-test/movies/Test Movie (2024)".to_string(),
                display_name: "Test Movie (2024)".to_string(),
                query: "Test Movie".to_string(),
                year_hint: Some(2024),
                reason_code: "title_binding_check".to_string(),
                error_message: Some("no exact metadata match".to_string()),
                search_attempts: vec![LibraryScanUnmatchedSearchAttempt {
                    query: "Test Movie".to_string(),
                    result_count: 1,
                    top_results: vec!["Test Movie (2024)".to_string()],
                }],
                created_at: now.clone(),
                updated_at: now,
            };

            let id = LibraryScanUnmatchedItemRepository::upsert_library_scan_unmatched_item(
                &store, &item,
            )
            .await?;
            assert_eq!(id, item.id);

            let stored = LibraryScanUnmatchedItemRepository::get_library_scan_unmatched_item(
                &store, &item.id,
            )
            .await?
            .expect("stored unmatched item");
            assert_eq!(stored.title_id, item.title_id);
            assert_eq!(stored.search_attempts, item.search_attempts);

            let listed = LibraryScanUnmatchedItemRepository::list_library_scan_unmatched_items(
                &store,
                Some(MediaFacet::Movie),
                Some(item.scan_root.as_str()),
                Some(PendingImportStatus::Pending),
                10,
                0,
            )
            .await?;
            assert_eq!(listed.len(), 1, "expected one unmatched scan item");
            assert_eq!(listed[0].title_id, item.title_id);

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_anime_backfill_queries_match_sqlite_title_external_id_semantics()
    -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL anime backfill parity test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let catalog = super::super::PostgresCatalogStore::new(&services);

            let mut missing_anidb = sample_title("pg-anime-missing-anidb");
            missing_anidb.facet = MediaFacet::Anime;
            missing_anidb.library_id = "anime_default_library".to_string();
            missing_anidb.slug = Some("pg-anime-missing-anidb".to_string());
            missing_anidb.external_ids = vec![ExternalId {
                source: "tvdb".to_string(),
                value: "1000".to_string(),
            }];
            TitleRepository::create(&catalog, missing_anidb.clone()).await?;

            let mut has_anidb = sample_title("pg-anime-has-anidb");
            has_anidb.facet = MediaFacet::Anime;
            has_anidb.library_id = "anime_default_library".to_string();
            has_anidb.slug = Some("pg-anime-has-anidb".to_string());
            has_anidb.external_ids = vec![
                ExternalId {
                    source: "tvdb".to_string(),
                    value: "2000".to_string(),
                },
                ExternalId {
                    source: "anidb".to_string(),
                    value: "3000".to_string(),
                },
            ];
            TitleRepository::create(&catalog, has_anidb.clone()).await?;

            let scoped_missing =
                TitleRepository::list_anime_title_ids_missing_anibridge_scoped_external_ids(
                    &catalog, 10,
                )
                .await?;
            assert!(
                scoped_missing.contains(&missing_anidb.id),
                "anime title with TVDB and no AniBridge scoped IDs should be queued"
            );
            assert!(
                scoped_missing.contains(&has_anidb.id),
                "title-level AniDB should not suppress AniBridge scoped-ID backfill"
            );

            let title_anidb_missing =
                TitleRepository::list_anime_title_ids_missing_title_anidb_external_ids(
                    &catalog, 10,
                )
                .await?;
            assert!(
                title_anidb_missing.contains(&missing_anidb.id),
                "anime title with TVDB and no title-level AniDB should be queued"
            );
            assert!(
                !title_anidb_missing.contains(&has_anidb.id),
                "anime title with title-level AniDB should not be queued"
            );

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    #[tokio::test]
    async fn postgres_set_grants_for_user_is_idempotent_under_concurrency() -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!("skipping PostgreSQL concurrency test; SCRYER_TEST_POSTGRES_URL is not set");
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let catalog = super::super::PostgresCatalogStore::new(&services);
            let user =
                UserRepository::create(&catalog, User::new_admin("admin-concurrency")).await?;
            let user_id = user.id.clone();
            let permissions = LibraryPermissionMask::from_permissions([
                LibraryPermission::View,
                LibraryPermission::ManageTitles,
                LibraryPermission::ResolveImports,
                LibraryPermission::ManageLibrary,
                LibraryPermission::Request,
                LibraryPermission::AutoApproveRequests,
            ]);

            let mut tasks = JoinSet::new();
            for _ in 0..8 {
                let catalog = catalog.clone();
                let user_id = user_id.clone();
                tasks.spawn(async move {
                    LibraryRepository::set_grants_for_user(
                        &catalog,
                        &user_id,
                        vec![LibraryGrant {
                            user_id: user_id.clone(),
                            library_id: "movie_default_library".to_string(),
                            permissions,
                        }],
                    )
                    .await
                });
            }

            while let Some(result) = tasks.join_next().await {
                result
                    .map_err(|error| AppError::Repository(error.to_string()))?
                    .map_err(|error| AppError::Repository(error.to_string()))?;
            }

            let grants = LibraryRepository::permission_masks_for_user(&catalog, &user.id).await?;
            assert_eq!(grants.len(), 1, "expected exactly one library grant row");
            assert_eq!(grants[0].library_id, "movie_default_library");
            assert_eq!(grants[0].permissions, permissions);

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin_pool)
            .await;
        admin_pool.close().await;
        if let Err(error) = cleanup {
            return Err(AppError::Repository(format!(
                "failed to drop test schema {schema}: {error}"
            )));
        }
        result
    }

    fn postgres_url_with_search_path(raw_url: &str, schema: &str) -> AppResult<String> {
        let mut url = url::Url::parse(raw_url)
            .map_err(|error| AppError::Validation(format!("invalid postgres test URL: {error}")))?;
        url.query_pairs_mut()
            .append_pair("options", &format!("-csearch_path={schema}"));
        Ok(url.to_string())
    }

    fn next_test_schema_name() -> String {
        format!(
            "scryer_test_{}_{}",
            std::process::id(),
            Id::new().0.replace('-', "_")
        )
    }

    fn postgres_parity_index_names_from_source() -> BTreeSet<String> {
        std::fs::read_to_string(postgres_parity_index_migration_path())
            .expect("read PostgreSQL parity secondary-index migration")
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                trimmed
                    .strip_prefix("CREATE UNIQUE INDEX IF NOT EXISTS ")
                    .or_else(|| trimmed.strip_prefix("CREATE INDEX IF NOT EXISTS "))
                    .and_then(|rest| rest.split_whitespace().next())
                    .map(str::to_string)
            })
            .collect()
    }

    fn postgres_parity_index_migration_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../scryer/src/db/postgres/migrations/0109_parity_secondary_indexes.sql")
    }

    async fn assert_postgres_runtime_schema_columns(pool: &sqlx::PgPool) -> AppResult<()> {
        let rows = sqlx::query(
            "SELECT table_name, column_name
               FROM information_schema.columns
              WHERE table_schema = current_schema()",
        )
        .fetch_all(pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

        let actual_columns: BTreeSet<(String, String)> = rows
            .into_iter()
            .map(|row| {
                Ok::<_, AppError>((
                    row.try_get("table_name")
                        .map_err(|error| AppError::Repository(error.to_string()))?,
                    row.try_get("column_name")
                        .map_err(|error| AppError::Repository(error.to_string()))?,
                ))
            })
            .collect::<AppResult<_>>()?;
        let actual_tables: BTreeSet<String> = actual_columns
            .iter()
            .map(|(table, _)| table.clone())
            .collect();
        let expected_columns = postgres_0115_baseline_columns();

        let mut missing_columns = Vec::new();
        for (table, columns) in &expected_columns {
            for column in columns {
                if !actual_columns.contains(&(table.clone(), column.clone())) {
                    missing_columns.push(format!("{table}.{column}"));
                }
            }
        }

        assert!(
            missing_columns.is_empty(),
            "expected PostgreSQL blank install to include every 0115 baseline column; missing {missing_columns:?}"
        );

        let unexpected_columns: Vec<String> = actual_columns
            .iter()
            .filter(|(table, column)| {
                expected_columns
                    .get(table)
                    .is_some_and(|columns| !columns.contains(column))
            })
            .map(|(table, column)| format!("{table}.{column}"))
            .collect();

        assert!(
            unexpected_columns.is_empty(),
            "PostgreSQL blank install exposes columns outside the 0115 baseline: {unexpected_columns:?}"
        );

        for removed_table in [
            "subtitle_providers",
            "import_artifacts",
            "job_runs",
            "quality_profiles_json",
            "mediarr_schema_migrations",
        ] {
            assert!(
                !actual_tables.contains(removed_table),
                "PostgreSQL blank install still exposes removed table {removed_table}"
            );
        }

        Ok(())
    }

    fn postgres_0115_baseline_columns() -> BTreeMap<String, BTreeSet<String>> {
        parse_create_table_columns(include_str!(
            "../../../scryer/src/db/postgres/baselines/0115_baseline.sql"
        ))
    }

    fn parse_create_table_columns(sql: &str) -> BTreeMap<String, BTreeSet<String>> {
        let mut schema = BTreeMap::<String, BTreeSet<String>>::new();
        let mut current_table: Option<String> = None;
        for line in sql.lines() {
            let trimmed = line.trim();
            if let Some(table) = current_table.as_ref() {
                if trimmed == ");" {
                    current_table = None;
                    continue;
                }
                if trimmed.is_empty()
                    || trimmed.starts_with("CONSTRAINT ")
                    || trimmed.starts_with("PRIMARY ")
                    || trimmed.starts_with("UNIQUE ")
                    || trimmed.starts_with("FOREIGN ")
                    || trimmed.starts_with("CHECK ")
                {
                    continue;
                }
                if let Some(column) = trimmed
                    .trim_end_matches(',')
                    .split_whitespace()
                    .next()
                    .map(|value| value.trim_matches('"').to_string())
                {
                    schema.entry(table.clone()).or_default().insert(column);
                }
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix("CREATE TABLE ") {
                let table = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches('(')
                    .trim_start_matches("public.")
                    .to_string();
                schema.entry(table.clone()).or_default();
                current_table = Some(table);
            }
        }
        schema
    }

    async fn assert_schema_parity_owned_tables_match_sqlite(
        postgres_pool: &sqlx::PgPool,
    ) -> AppResult<()> {
        let sqlite = crate::sqlite_services::SqliteServices::new_with_mode(
            "sqlite://:memory:",
            MigrationMode::Apply,
        )
        .await?;

        let sqlite_schema = sqlite_table_columns(sqlite.pool()).await?;
        let postgres_schema = postgres_table_columns(postgres_pool).await?;
        let parity_tables = [
            "titles",
            "indexers",
            "download_clients",
            "subtitle_provider_configs",
            "rule_sets",
            "post_processing_scripts",
            "post_processing_script_runs",
            "plugin_installations",
            "plugin_catalog_sources",
            "plugin_catalog_status",
        ];

        let mut mismatches = Vec::new();
        for table in parity_tables {
            let sqlite_columns = sqlite_schema.get(table).cloned().unwrap_or_default();
            let postgres_columns = postgres_schema.get(table).cloned().unwrap_or_default();
            if sqlite_columns != postgres_columns {
                mismatches.push(format!(
                    "{table}: sqlite={sqlite_columns:?} postgres={postgres_columns:?}"
                ));
            }
        }

        assert!(
            mismatches.is_empty(),
            "SQLite/PostgreSQL logical schema parity mismatch: {mismatches:#?}"
        );
        Ok(())
    }

    async fn sqlite_table_columns(
        pool: &sqlx::SqlitePool,
    ) -> AppResult<BTreeMap<String, BTreeSet<String>>> {
        let rows = sqlx::query(
            "SELECT m.name AS table_name, p.name AS column_name
               FROM sqlite_master m
               JOIN pragma_table_info(m.name) p
              WHERE m.type = 'table'
                AND m.name NOT LIKE 'sqlite_%'",
        )
        .fetch_all(pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let mut schema = BTreeMap::new();
        for row in rows {
            let table_name: String = row
                .try_get("table_name")
                .map_err(|error| AppError::Repository(error.to_string()))?;
            let column_name: String = row
                .try_get("column_name")
                .map_err(|error| AppError::Repository(error.to_string()))?;
            schema
                .entry(table_name)
                .or_insert_with(BTreeSet::new)
                .insert(column_name);
        }
        Ok(schema)
    }

    async fn postgres_table_columns(
        pool: &sqlx::PgPool,
    ) -> AppResult<BTreeMap<String, BTreeSet<String>>> {
        let rows = sqlx::query(
            "SELECT table_name, column_name
               FROM information_schema.columns
              WHERE table_schema = current_schema()",
        )
        .fetch_all(pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let mut schema = BTreeMap::new();
        for row in rows {
            let table_name: String = row
                .try_get("table_name")
                .map_err(|error| AppError::Repository(error.to_string()))?;
            let column_name: String = row
                .try_get("column_name")
                .map_err(|error| AppError::Repository(error.to_string()))?;
            schema
                .entry(table_name)
                .or_insert_with(BTreeSet::new)
                .insert(column_name);
        }
        Ok(schema)
    }

    fn sample_title(id: &str) -> Title {
        Title {
            id: id.to_string(),
            library_id: "movie_default_library".to_string(),
            name: "Postgres Blank Install Smoke".to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: Vec::new(),
            external_ids: Vec::new(),
            created_by: None,
            created_at: chrono::Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            slug: Some("postgres-blank-install-smoke".to_string()),
            imdb_id: None,
            runtime_minutes: None,
            genres: Vec::new(),
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: Vec::new(),
            tagged_aliases: Vec::new(),
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }
}
