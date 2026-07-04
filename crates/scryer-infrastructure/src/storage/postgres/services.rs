use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use scryer_application::{AppError, AppResult};
use sqlx::ConnectOptions;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tracing::log::LevelFilter;

use crate::encryption::{EncryptionKey, load_existing_encryption_key_without_generation};
use crate::migrations::MigrationHookContext;
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
        Self::new_with_mode_and_data_dir(database_url, migration_mode, None).await
    }

    pub async fn new_with_mode_and_data_dir(
        database_url: impl AsRef<str>,
        migration_mode: MigrationMode,
        data_dir: Option<PathBuf>,
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

        let migration_encryption_key = load_existing_encryption_key_without_generation(data_dir)
            .map_err(AppError::Repository)?;
        super::migrations::run_migrations_with_hook_context(
            &pool,
            migration_mode,
            MigrationHookContext {
                encryption_key: migration_encryption_key,
            },
        )
        .await?;

        Ok(Self {
            pool,
            encryption_key: Arc::new(RwLock::new(None)),
        })
    }

    pub fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }

    pub fn datastore(&self) -> crate::queries::sql_runtime::StoreDatastore {
        crate::queries::sql_runtime::StoreDatastore::Postgres {
            pool: self.pool.clone(),
        }
    }

    pub fn encryption_key_state(&self) -> Arc<RwLock<Option<EncryptionKey>>> {
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
        InsertMediaFileInput, LibraryRepository, LibraryScanUnmatchedItem,
        LibraryScanUnmatchedItemRepository, LibraryScanUnmatchedSearchAttempt, MediaFileAnalysis,
        MediaFileRepository, PendingImportStatus, QualityProfileRepository, SettingsRepository,
        ShowRepository, SystemInfoProvider, TitleImageKind, TitleImageRepository,
        TitleImageSourceResult, TitleImageVariantRecord, TitleMetadataUpdate, TitleRepository,
        UserRepository, default_quality_profile_for_search,
    };
    use scryer_domain::{
        Collection, CollectionType, ExternalId, Id, Library, LibraryGrant, LibraryPermission,
        LibraryPermissionMask, MediaFacet, Title, User,
    };
    use sqlx::Row;
    use tokio::task::JoinSet;

    use crate::{LibraryStore, SettingDefinitionSeed, TitleStore, UserStore};

    fn title_store(services: &PostgresServices) -> TitleStore {
        TitleStore::new(services.datastore())
    }

    fn library_store(services: &PostgresServices) -> LibraryStore {
        LibraryStore::new(services.datastore())
    }

    fn settings_store(services: &PostgresServices) -> crate::SettingsStore {
        crate::SettingsStore::new(services.datastore(), services.encryption_key_state())
    }

    fn user_store(services: &PostgresServices) -> UserStore {
        UserStore::new(services.datastore())
    }

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

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let catalog = title_store(&services);
            let images = crate::TitleImageStore::new(services.datastore());
            let settings = settings_store(&services);
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
            let default_library_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM libraries WHERE is_default = TRUE")
                    .fetch_one(services.pool())
                    .await
                    .map_err(|error| AppError::Repository(error.to_string()))?;
            assert_eq!(
                default_library_count, 3,
                "expected PostgreSQL blank install to seed canonical default libraries"
            );
            let default_root_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM library_roots
                 WHERE is_default = TRUE
                   AND library_id IN ('movie_default_library', 'series_default_library', 'anime_default_library')",
            )
            .fetch_one(services.pool())
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
            assert_eq!(
                default_root_count, 3,
                "expected PostgreSQL blank install to seed canonical default library roots"
            );

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
                        canonical_subject_key: None,
                        name: None,
                        year: Some(2024),
                        overview: Some("postgres blank install smoke".to_string()),
                        poster_url: Some("https://example.com/poster.jpg".to_string()),
                        background_url: Some("https://example.com/fanart.jpg".to_string()),
                        sort_title: Some("Postgres Blank Install Smoke".to_string()),
                        slug: Some("postgres-blank-install-smoke".to_string()),
                        imdb_id: Some("tt1234567".to_string()),
                        runtime_minutes: Some(123),
                        popularity: None,
                        genres: vec!["Drama".to_string()],
                        canonical_tags: vec![],
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
                        ratings: None,
                        extra_external_ids: Vec::new(),
                        extra_tags: Vec::new(),
                    },
                )
                .await?;
            catalog
                .clear_title_metadata_hydration_retry_state(&title.id)
                .await?;

            let poster_tasks = images
                .list_title_image_refresh_work(10, &[])
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
                .upsert_title_image_source_result(
                    &title.id,
                    TitleImageSourceResult {
                        kind: TitleImageKind::Poster,
                        source_url: "https://example.com/poster.jpg".to_string(),
                        source_etag: Some("source-etag".to_string()),
                        source_last_modified: Some("Wed, 14 May 2026 03:00:00 GMT".to_string()),
                        source_format: "jpeg".to_string(),
                        source_width: 1200,
                        source_height: 1800,
                        variants: vec![TitleImageVariantRecord {
                            variant_key: "w250".to_string(),
                            format: "avif".to_string(),
                            width: 250,
                            height: 375,
                            bytes: vec![5, 6, 7, 8],
                            digest: "blake3:poster-thumb".to_string(),
                        }],
                    },
                    None,
                )
                .await?;

            let original_blob = images
                .get_title_image_blob(&title.id, TitleImageKind::Poster, "original")
                .await?;
            assert!(original_blob.is_none());

            let thumb_blob = images
                .get_title_image_blob(&title.id, TitleImageKind::Poster, "w250")
                .await?
                .ok_or_else(|| {
                    AppError::Repository(
                        "expected PostgreSQL blank install to persist title image variants"
                            .to_string(),
                    )
                })?;
            assert_eq!(thumb_blob.content_type, "image/avif");
            assert_eq!(thumb_blob.etag, "blake3:poster-thumb");
            assert_eq!(thumb_blob.bytes, vec![5, 6, 7, 8]);

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
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

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let settings = settings_store(&services);

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

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
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
    async fn postgres_library_scoped_download_client_routing_round_trips_explicit_json()
    -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL library download client routing test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let settings = settings_store(&services);

            settings
                .batch_ensure_setting_definitions(vec![SettingDefinitionSeed {
                    category: "media".to_string(),
                    scope: "system".to_string(),
                    key_name: "download_client.routing".to_string(),
                    data_type: "json".to_string(),
                    default_value_json: "{}".to_string(),
                    is_sensitive: false,
                    validation_json: None,
                }])
                .await?;

            let library_id = "series_default_library";
            let value_json = serde_json::json!({
                "weaver": {
                    "enabled": true,
                    "category": "series",
                    "recentQueuePriority": "high",
                    "olderQueuePriority": "normal",
                    "removeCompleted": true,
                    "removeFailed": false
                }
            })
            .to_string();

            SettingsRepository::upsert_setting_json(
                &settings,
                "system",
                "download_client.routing",
                Some(library_id.to_string()),
                value_json.clone(),
                "test",
                None,
            )
            .await?;

            let explicit = SettingsRepository::get_setting_json_explicit(
                &settings,
                "system",
                "download_client.routing",
                Some(library_id.to_string()),
            )
            .await?;
            assert_eq!(explicit.as_deref(), Some(value_json.as_str()));

            let default_lookup = SettingsRepository::get_setting_json(
                &settings,
                "system",
                "download_client.routing",
                Some("another_library".to_string()),
            )
            .await?;
            assert_eq!(default_lookup.as_deref(), Some("{}"));

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
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

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let quality_profiles = crate::QualityProfileStore::new(services.datastore());
            let profiles = vec![default_quality_profile_for_search()];

            quality_profiles
                .replace_quality_profiles("system", None, profiles.clone())
                .await?;

            let stored = quality_profiles
                .list_quality_profiles("system", None)
                .await?;
            assert_eq!(stored, profiles);

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
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

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
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

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
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
    async fn postgres_media_file_store_uses_sqlite_canonical_statuses() -> AppResult<()> {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL media-file store test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return Ok(());
        };

        let admin_pool = sqlx::PgPool::connect(&raw_url).await.map_err(|error| {
            AppError::Repository(format!("failed to connect to postgres: {error}"))
        })?;
        let schema = next_test_schema_name();

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let catalog = title_store(&services);
            let media_files = crate::MediaFileStore::new(services.datastore());

            let title = sample_title("pg-media-file-store");
            TitleRepository::create(&catalog, title.clone()).await?;

            let file_id = media_files
                .insert_media_file(&InsertMediaFileInput {
                    title_id: title.id.clone(),
                    file_path: "/library/Movie.Title.2026.mkv".to_string(),
                    size_bytes: 8_192,
                    quality_label: Some("720p".to_string()),
                    grabbed_at: Some("2026-05-01T00:00:00Z".to_string()),
                    ..Default::default()
                })
                .await?;
            media_files
                .insert_media_file(&InsertMediaFileInput {
                    title_id: title.id.clone(),
                    file_path: "/library/.scryer-recycle/old/Movie.Title.2026.mkv".to_string(),
                    size_bytes: 1,
                    quality_label: Some("2160p".to_string()),
                    ..Default::default()
                })
                .await?;

            let listed = media_files.list_media_files_for_title(&title.id).await?;
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].id, file_id);
            assert_eq!(listed[0].scan_status, "imported");
            assert_eq!(
                listed[0].grabbed_at.as_deref(),
                Some("2026-05-01T00:00:00+00:00")
            );

            media_files
                .update_media_file_analysis(
                    &file_id,
                    MediaFileAnalysis {
                        video_codec: Some(
                            scryer_application::VideoCodec::parse("hevc").expect("parse codec"),
                        ),
                        video_width: Some(3840),
                        video_height: Some(2160),
                        video_bitrate_kbps: None,
                        video_bit_depth: Some(10),
                        video_hdr_format: Some("HDR10".to_string()),
                        video_frame_rate: Some("23.976".to_string()),
                        video_profile: Some("Main 10".to_string()),
                        audio_codec: Some("aac".to_string()),
                        audio_profile: Some("LC".to_string()),
                        audio_channels: Some(2),
                        audio_bitrate_kbps: None,
                        audio_languages: vec!["eng".to_string()],
                        audio_streams: vec![],
                        subtitle_languages: vec![],
                        subtitle_codecs: vec![],
                        subtitle_streams: vec![],
                        has_multiaudio: false,
                        duration_seconds: Some(7200),
                        num_chapters: Some(12),
                        container_format: Some("matroska".to_string()),
                    },
                )
                .await?;
            let stored = media_files
                .get_media_file_by_id(&file_id)
                .await?
                .expect("stored media file");
            assert_eq!(stored.scan_status, "scanned");
            assert_eq!(stored.audio_languages, vec!["eng".to_string()]);

            media_files
                .mark_scan_failed(&file_id, "probe failed")
                .await?;
            let failed = media_files
                .get_media_file_by_id(&file_id)
                .await?
                .expect("stored media file after failure");
            assert_eq!(failed.scan_status, "scan_failed");

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
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

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let store = crate::LibraryScanUnmatchedStore::new(services.datastore());
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

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
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

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let catalog = title_store(&services);

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

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
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

        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create test schema: {error}"))
            })?;

        let result = async {
            let schema_url = postgres_url_with_search_path(&raw_url, &schema)?;
            let services =
                PostgresServices::new_with_mode(schema_url, MigrationMode::Apply).await?;
            let users = user_store(&services);
            let libraries = library_store(&services);
            let now = chrono::Utc::now();
            let movie_library = Library {
                id: "movie_default_library".to_string(),
                facet: MediaFacet::Movie,
                name: "Default movie".to_string(),
                slug: "movies".to_string(),
                is_default: true,
                roots: Vec::new(),
                created_at: now,
                updated_at: now,
            };
            LibraryRepository::create(&libraries, movie_library, Vec::new()).await?;
            let user = UserRepository::create(&users, User::new_admin("admin-concurrency")).await?;
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
                let libraries = libraries.clone();
                let user_id = user_id.clone();
                tasks.spawn(async move {
                    LibraryRepository::set_grants_for_user(
                        &libraries,
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

            let grants = LibraryRepository::permission_masks_for_user(&libraries, &user.id).await?;
            assert_eq!(grants.len(), 1, "expected exactly one library grant row");
            assert_eq!(grants[0].library_id, "movie_default_library");
            assert_eq!(grants[0].permissions, permissions);

            services.pool().close().await;
            Ok::<_, AppError>(())
        }
        .await;

        let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
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
        std::fs::read_to_string(postgres_0122_baseline_path())
            .expect("read PostgreSQL 0122 baseline")
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                trimmed
                    .strip_prefix("CREATE UNIQUE INDEX ")
                    .or_else(|| trimmed.strip_prefix("CREATE INDEX "))
                    .and_then(|rest| rest.split_whitespace().next())
                    .map(str::to_string)
            })
            .collect()
    }

    fn postgres_0122_baseline_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../scryer/src/db/postgres/baselines/0122_baseline.sql")
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
        let expected_columns = postgres_0122_baseline_columns();

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
            "expected PostgreSQL blank install to include every 0122 baseline column; missing {missing_columns:?}"
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
            "PostgreSQL blank install exposes columns outside the 0122 baseline: {unexpected_columns:?}"
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

    fn postgres_0122_baseline_columns() -> BTreeMap<String, BTreeSet<String>> {
        let mut columns = parse_create_table_columns(include_str!(
            "../../../../scryer/src/db/postgres/baselines/0122_baseline.sql"
        ));
        columns
            .entry("titles".to_string())
            .or_default()
            .insert("catalog_sort_key".to_string());
        columns
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
            canonical_tags: vec![],
            external_ids: Vec::new(),
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/movies"),
            created_by: None,
            created_at: chrono::Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            catalog_sort_key: String::new(),
            slug: Some("postgres-blank-install-smoke".to_string()),
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
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
