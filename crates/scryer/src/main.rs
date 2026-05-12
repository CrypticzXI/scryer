// async-graphql schema expansion exceeded the default macro recursion depth.
#![recursion_limit = "256"]

mod admin_routes;
mod backup_routes;
mod base_path;
mod init;
mod log_buffer;
mod middleware;
mod rate_limit;
mod settings_bootstrap;
mod splash;
mod ui_assets;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Path as AxumPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use scryer_application::{
    AppServices, AppUseCase, DownloadClientPluginProvider, FacetRegistry, HISTORY_KEEP_FOREVER_KEY,
    HISTORY_RETENTION_DAYS_KEY, IndexerPluginProvider, MovieFacetHandler,
    NotificationPluginProvider, PluginInstallationRepository, RUNTIME_PLUGIN_LOAD_CONCURRENCY,
    RuntimePluginLoad, SeriesFacetHandler, SubtitlePluginProvider, TitleImageKind,
    TitleImageRepository, load_runtime_plugin_from_persisted_installation_payload,
    start_background_acquisition_poller, start_background_banner_loop,
    start_background_download_delete_poller, start_background_fanart_loop,
    start_background_library_refresh_loop, start_background_manual_import_poller,
    start_background_poster_loop, start_background_subtitle_poller,
    start_background_title_hydration_loop, start_download_queue_poller,
    start_notification_dispatcher, tracked_downloads::TrackedDownloadHandle,
};
use scryer_infrastructure::{
    FileSystemLibraryRenamer, FileSystemLibraryScanner, FileSystemStagedNzbStore,
    MetadataGatewayClient, MigrationMode, MultiIndexerSearchClient, NzbgetDownloadClient,
    PrioritizedDownloadClientRouter, SmgEnrollmentConfig, SqliteCatalogStore, SqliteConfigStore,
    SqliteCustomizationStore, SqliteLibraryStateStore, SqliteNotificationStore, SqliteReleaseStore,
    SqliteServices, SqliteSettingsStore, SqliteTitleImageProcessor, SqliteWorkflowStore,
    WeaverDownloadClient, start_weaver_subscription_bridge,
};
use scryer_interface::context::{AuthRuntimeStateHandle, AuthRuntimeStateSnapshot};
use scryer_interface::{LogBuffer, build_schema_with_log_buffer};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tower_http::compression::CompressionLayer;

use admin_routes::{
    AdminSettingsQuery, admin_migrations_handler, admin_settings_list, bootstrap_admin_password,
    seed_indexer_configs_from_env,
};
use backup_routes::{
    BackupRouteState, download_backup_handler, finalize_pending_restore_if_present,
    setup_restore_apply_handler, setup_restore_inspect_handler,
};
use base_path::BasePath;
use middleware::{
    AuthState, CorsConfig, cors_handler, graphql_handler, graphql_ws_handler, health_handler,
    rate_limit_http_api,
};
use rate_limit::ScryerRateLimiter;
use settings_bootstrap::{
    MOVIES_PATH_KEY, SERIES_PATH_KEY, extract_pending_migration_ids, load_service_runtime_settings,
    migrate_legacy_download_client_default_category_settings,
    migrate_legacy_download_client_routing_settings, normalize_media_path_setting,
    normalize_quality_profile_settings, parse_migration_mode, seed_service_setting_definitions,
    seed_service_settings_from_environment,
};
use splash::{BootstrapStatus, SplashState, build_splash_router};
use ui_assets::{UiAssetMode, ui_asset_mode, ui_fallback};

include!(concat!(env!("OUT_DIR"), "/smg_build_assets.rs"));

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn plugin_type_belongs_to_indexer_family(plugin_type: &str) -> bool {
    matches!(
        plugin_type,
        "indexer" | "usenet_indexer" | "torrent_indexer"
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AuthModeConfig {
    env_override_form_login_enabled: Option<bool>,
    env_override_description: Option<String>,
    used_legacy_dev_auto_login: bool,
}

impl AuthModeConfig {
    fn env_override_active(&self) -> bool {
        self.env_override_form_login_enabled.is_some()
    }

    fn effective_form_login_enabled(&self, saved_form_login_enabled: bool) -> bool {
        self.env_override_form_login_enabled
            .unwrap_or(saved_form_login_enabled)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VersionLifecycle {
    FirstRun,
    Unchanged,
    Upgraded,
}

fn log_smg_version_incompatibility(
    incompat: &scryer_infrastructure::smg_enrollment::VersionIncompatible,
) {
    let env = if std::path::Path::new("/.dockerenv").exists() {
        "docker"
    } else {
        "binary"
    };
    let blocked = incompat.status.eq_ignore_ascii_case("blocked");
    let level_message = if blocked {
        "SMG upgrade required"
    } else {
        "SMG upgrade recommended"
    };

    if blocked {
        tracing::error!(
            status = %incompat.status,
            minimum_version = %incompat.minimum_version,
            your_version = %incompat.your_version,
            upgrade_deadline = ?incompat.upgrade_deadline,
            "{level_message}: {}",
            incompat.message
        );
    } else {
        tracing::warn!(
            status = %incompat.status,
            minimum_version = %incompat.minimum_version,
            your_version = %incompat.your_version,
            upgrade_deadline = ?incompat.upgrade_deadline,
            "{level_message}: {}",
            incompat.message
        );
    }

    if env == "docker" {
        if blocked {
            tracing::error!(
                "To upgrade, pull the latest image and restart:\n  docker pull ghcr.io/scryer-media/scryer:latest\n  docker compose up -d"
            );
        } else {
            tracing::warn!(
                "To upgrade, pull the latest image and restart:\n  docker pull ghcr.io/scryer-media/scryer:latest\n  docker compose up -d"
            );
        }
    } else if blocked {
        tracing::error!(
            "Download the latest release from:\n  https://github.com/scryer-media/scryer/releases/latest"
        );
    } else {
        tracing::warn!(
            "Download the latest release from:\n  https://github.com/scryer-media/scryer/releases/latest"
        );
    }
}

#[tokio::main]
async fn main() {
    // Phase 1: Extract --data-dir from args before subcommand dispatch.
    let mut args: Vec<String> = std::env::args().collect();
    let data_dir_override = extract_data_dir(&mut args);

    // Phase 2: Handle CLI subcommands before any startup work.
    // args[0] is the binary name; subcommand (if any) is args[1].
    if let Some(arg) = args.get(1) {
        match arg.as_str() {
            "init" => {
                init::run_init(args);
                return;
            }
            "--generate-key" => {
                let key = scryer_infrastructure::encryption::EncryptionKey::generate();
                println!("{}", key.to_base64());
                return;
            }
            "--version" | "-V" => {
                println!("scryer {VERSION}");
                return;
            }
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!("usage: scryer [--data-dir <path>] [init | --generate-key | --version]");
                std::process::exit(1);
            }
        }
    }

    let data_dir = resolve_data_dir(data_dir_override.as_deref());

    load_env_file(Some(&data_dir), false);

    // Install ring as the default rustls crypto provider (needed for TLS support)
    let _ = rustls::crypto::ring::default_provider().install_default();

    let pre_restore_db_path = std::env::var("SCRYER_DB_PATH")
        .unwrap_or_else(|_| format!("sqlite://{}", data_dir.join("scryer.db").display()));

    let log_ring_buffer = log_buffer::LogRingBuffer::with_default_capacity();

    {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;

        let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

        let stdout_layer = tracing_subscriber::fmt::layer();
        let buffer_layer = tracing_subscriber::fmt::layer()
            .with_writer(log_buffer::LogBufferWriter::new(log_ring_buffer.clone()))
            .with_ansi(false);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(stdout_layer)
            .with(buffer_layer)
            .init();
    }

    if let Err(error) = finalize_pending_restore_if_present(&data_dir, &pre_restore_db_path) {
        tracing::error!(error = %error, "failed to finalize pending restore");
        std::process::exit(1);
    }

    load_env_file(Some(&data_dir), true);

    let db_path = std::env::var("SCRYER_DB_PATH")
        .unwrap_or_else(|_| format!("sqlite://{}", data_dir.join("scryer.db").display()));
    // Ensure the database directory exists regardless of how db_path was resolved.
    if let Some(path) = db_path.strip_prefix("sqlite://")
        && let Some(parent) = std::path::Path::new(path).parent()
    {
        let _ = std::fs::create_dir_all(parent);
    }
    let jwt_issuer = std::env::var("SCRYER_JWT_ISSUER").unwrap_or_else(|_| "scryer".to_string());
    let jwt_access_ttl_seconds = parse_env_u64("SCRYER_JWT_ACCESS_TTL_SECONDS", 86_400);
    let migration_mode = parse_migration_mode(std::env::var("SCRYER_DB_MIGRATION_MODE").ok());
    let bind = std::env::var("SCRYER_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let base_path = BasePath::from_env();

    // Install Prometheus metrics recorder when enabled.
    // The `metrics` crate uses a global facade — once installed, `metrics::counter!()`
    // calls from any crate resolve to this recorder. When not installed, they are no-ops.
    let metrics_handle = if std::env::var("SCRYER_METRICS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        let handle = metrics_exporter_prometheus::PrometheusBuilder::new()
            .install_recorder()
            .expect("failed to install prometheus metrics recorder");
        tracing::info!("prometheus metrics enabled at /metrics");
        Some(handle)
    } else {
        None
    };

    tracing::info!(version = VERSION, "starting scryer");

    // ValidateOnly mode: check for pending migrations and exit immediately (no server).
    if matches!(migration_mode, MigrationMode::ValidateOnly) {
        run_validate_only(&db_path, migration_mode).await;
        return;
    }

    // Read TLS config from env vars (available before DB bootstrap).
    let tls_cert_path = normalize_env_option("SCRYER_TLS_CERT");
    let tls_key_path = normalize_env_option("SCRYER_TLS_KEY");

    // Create the watch channel for bootstrap status communication.
    let (status_tx, status_rx) = watch::channel(BootstrapStatus::Migrating);
    let splash_state = SplashState { status_rx };
    let cors = CorsConfig::from_env();
    let splash_app = build_splash_router(splash_state, cors.clone(), base_path.clone());

    let cors_allow_all = cors.allow_all || cors.allowed_origins.iter().any(|origin| origin == "*");
    if cors_allow_all {
        tracing::warn!("CORS configured with wildcard origin(s)");
    } else {
        tracing::info!(origins = ?cors.allowed_origins, "CORS configured with explicit origin list");
    }

    let addr: SocketAddr = bind.parse().expect("invalid bind address");
    let shutdown_token = CancellationToken::new();
    let startup_base_path = base_path.clone();

    // Spawn the full application bootstrap in the background.
    let bootstrap_shutdown = shutdown_token.clone();
    let bootstrap_bind = bind.clone();
    let runtime_handle = tokio::runtime::Handle::current();
    std::thread::Builder::new()
        .name("scryer-bootstrap".to_string())
        .spawn(move || {
            runtime_handle.block_on(async move {
                match bootstrap_application(
                    db_path,
                    migration_mode,
                    jwt_issuer,
                    jwt_access_ttl_seconds,
                    bootstrap_bind,
                    cors,
                    bootstrap_shutdown,
                    log_ring_buffer,
                    metrics_handle,
                    data_dir,
                )
                .await
                {
                    Ok(router) => {
                        let _ = status_tx.send(BootstrapStatus::Ready(router));
                    }
                    Err(error) => {
                        tracing::error!(error = %error, "application bootstrap failed");
                        let _ = status_tx.send(BootstrapStatus::Failed(error.to_string()));
                    }
                }
            });
        })
        .expect("failed to spawn bootstrap thread");

    // Start serving immediately — splash handlers delegate to the full app once ready.
    match (tls_cert_path, tls_key_path) {
        (Some(cert_path), Some(key_path)) => {
            let rustls_config =
                axum_server::tls_rustls::RustlsConfig::from_pem_file(&cert_path, &key_path)
                    .await
                    .unwrap_or_else(|error| {
                        panic!(
                            "failed to load TLS certificates (cert={}, key={}): {error}",
                            cert_path, key_path
                        );
                    });
            let handle = axum_server::Handle::new();
            let shutdown_handle = handle.clone();
            let shutdown_token_tls = shutdown_token.clone();
            tokio::spawn(async move {
                shutdown_signal(shutdown_token_tls).await;
                shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(5)));
            });
            tracing::info!("scryer service listening on {addr} with TLS");
            let url = format!("https://{addr}{}", startup_base_path.ui_root());
            tracing::info!("open the web UI at {url}");
            maybe_open_browser(&url);
            if let Err(error) = axum_server::bind_rustls(addr, rustls_config)
                .handle(handle)
                .serve(splash_app.into_make_service_with_connect_info::<SocketAddr>())
                .await
            {
                tracing::error!(error = %error, "TLS server failed");
                std::process::exit(1);
            }
        }
        (Some(_), None) | (None, Some(_)) => {
            panic!("both SCRYER_TLS_CERT and SCRYER_TLS_KEY must be set for TLS, or neither");
        }
        (None, None) => {
            let listener = TcpListener::bind(addr)
                .await
                .expect("failed to bind address");
            tracing::info!(
                "scryer service listening on {}",
                listener.local_addr().expect("bound addr")
            );
            let url = format!("http://{addr}{}", startup_base_path.ui_root());
            tracing::info!("open the web UI at {url}");
            maybe_open_browser(&url);
            if let Err(error) = axum::serve(
                listener,
                splash_app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown_signal(shutdown_token.clone()))
            .await
            {
                tracing::error!(error = %error, "server failed");
                std::process::exit(1);
            }
        }
    }
}

/// Runs the full application bootstrap: DB init, migrations, service construction, and router
/// building. Returns the fully-constructed Axum router or an error.
#[expect(clippy::too_many_arguments)]
async fn bootstrap_application(
    db_path: String,
    migration_mode: MigrationMode,
    jwt_issuer: String,
    jwt_access_ttl_seconds: u64,
    bind: String,
    cors: CorsConfig,
    shutdown_token: CancellationToken,
    log_ring_buffer: log_buffer::LogRingBuffer,
    metrics_handle: Option<metrics_exporter_prometheus::PrometheusHandle>,
    data_dir: PathBuf,
) -> Result<Router, Box<dyn std::error::Error + Send + Sync>> {
    let bootstrap_start = std::time::Instant::now();

    let t = std::time::Instant::now();
    let db = SqliteServices::new_with_mode(db_path.clone(), migration_mode)
        .await
        .map_err(|e| format!("failed to initialize sqlite services: {e}"))?;
    let bootstrap_settings_store = SqliteSettingsStore::new(&db);
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "database initialized");

    let t = std::time::Instant::now();
    seed_service_setting_definitions(bootstrap_settings_store.clone())
        .await
        .map_err(|e| format!("failed to seed service setting definitions: {e}"))?;
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "setting definitions seeded");

    // Bootstrap encryption master key (env > keystore > legacy DB migration > auto-generate).
    let t = std::time::Instant::now();
    let encryption_key =
        scryer_infrastructure::encryption::ensure_encryption_key(&db, Some(data_dir.clone()))
            .await
            .map_err(|e| format!("failed to ensure encryption master key: {e}"))?;

    // Activate encryption for all subsequent DB operations
    db.set_encryption_key(encryption_key)
        .await
        .map_err(|e| format!("failed to set encryption key on DB worker: {e}"))?;
    let migrated_indexers = db
        .migrate_legacy_indexer_config_sources()
        .await
        .map_err(|e| format!("failed to migrate legacy indexer config sources: {e}"))?;
    if migrated_indexers > 0 {
        tracing::info!(
            migrated = migrated_indexers,
            "migrated legacy indexer base/api fields into config_json"
        );
    }
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "encryption bootstrapped");

    // Detect version upgrades by comparing with last-run version stored in DB
    let _version_lifecycle = check_version_upgrade(bootstrap_settings_store.clone()).await;
    clear_legacy_history_retention_forever_override(bootstrap_settings_store.clone()).await;

    let t = std::time::Instant::now();
    if let Err(error) =
        seed_service_settings_from_environment(bootstrap_settings_store.clone()).await
    {
        tracing::warn!(
            error = %error,
            "failed to persist optional settings from environment"
        );
    }
    if let Err(error) =
        migrate_legacy_download_client_routing_settings(bootstrap_settings_store.clone()).await
    {
        tracing::warn!(
            error = %error,
            "failed to migrate legacy download client routing settings during bootstrap"
        );
    }

    if let Err(error) =
        migrate_legacy_download_client_default_category_settings(bootstrap_settings_store.clone())
            .await
    {
        tracing::warn!(
            error = %error,
            "failed to migrate legacy download client default category settings during bootstrap"
        );
    }
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "environment settings synced");

    let t = std::time::Instant::now();
    if let Err(error) = normalize_media_path_setting(
        bootstrap_settings_store.clone(),
        MOVIES_PATH_KEY.to_string(),
    )
    .await
    {
        tracing::warn!(
            error = %error,
            "failed to normalize media movies.path setting during bootstrap"
        );
    }

    if let Err(error) = normalize_media_path_setting(
        bootstrap_settings_store.clone(),
        SERIES_PATH_KEY.to_string(),
    )
    .await
    {
        tracing::warn!(
            error = %error,
            "failed to normalize media series.path setting during bootstrap"
        );
    }

    // Construct the facet registry early so scope IDs are available for settings bootstrap.
    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));
    let facet_registry = Arc::new(registry);

    if let Err(error) = normalize_quality_profile_settings(
        bootstrap_settings_store.clone(),
        facet_registry
            .facet_ids()
            .into_iter()
            .map(str::to_string)
            .collect(),
    )
    .await
    {
        tracing::warn!(
            error = %error,
            "failed to normalize quality profile settings during bootstrap"
        );
    }
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "settings normalized");

    let t = std::time::Instant::now();
    let runtime_settings = load_service_runtime_settings(bootstrap_settings_store.clone())
        .await
        .map_err(|e| format!("failed to load service runtime settings: {e}"))?;
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "runtime settings loaded");

    tracing::info!(elapsed_ms = %bootstrap_start.elapsed().as_millis(), "bootstrap complete");

    let catalog_store = Arc::new(SqliteCatalogStore::new(&db));
    let titles: Arc<dyn scryer_application::TitleRepository> = catalog_store.clone();
    let users: Arc<dyn scryer_application::UserRepository> = catalog_store.clone();
    let shows: Arc<dyn scryer_application::ShowRepository> = catalog_store.clone();
    let libraries: Arc<dyn scryer_application::LibraryRepository> = catalog_store;
    let config_store = Arc::new(SqliteConfigStore::new(&db));
    let release_store = Arc::new(SqliteReleaseStore::new(&db));
    let settings_store = Arc::new(SqliteSettingsStore::new(&db));
    let indexer_configs: Arc<dyn scryer_application::IndexerConfigRepository> =
        config_store.clone();
    let release_attempts: Arc<dyn scryer_application::ReleaseAttemptRepository> = release_store;
    let download_client_configs: Arc<dyn scryer_application::DownloadClientConfigRepository> =
        config_store.clone();
    let subtitle_provider_configs: Arc<dyn scryer_application::SubtitleProviderConfigRepository> =
        config_store.clone();
    let settings_for_router: Arc<dyn scryer_application::SettingsRepository> =
        settings_store.clone();
    let quality_profiles: Arc<dyn scryer_application::QualityProfileRepository> =
        settings_store.clone();
    let customization_store = Arc::new(SqliteCustomizationStore::new(&db));
    let staged_nzb_store = Arc::new(
        FileSystemStagedNzbStore::new_with_startup_purge(
            FileSystemStagedNzbStore::path_for_main_db(&db_path),
            true,
        )
        .await
        .map_err(|e| format!("failed to initialize staged nzb store: {e}"))?,
    );
    let staged_nzb_pipeline_limit = Arc::new(tokio::sync::Semaphore::new(4));
    bootstrap_plugin_installations(customization_store.as_ref())
        .await
        .map_err(|e| format!("failed to bootstrap plugin installations: {e}"))?;
    let (runtime_plugins, disabled_builtin_plugins) =
        load_runtime_plugin_state(customization_store.as_ref())
            .await
            .map_err(|e| format!("failed to load runtime plugin state: {e}"))?;
    let indexer_runtime_plugins = runtime_plugins
        .iter()
        .filter(|plugin| plugin_type_belongs_to_indexer_family(plugin.descriptor.plugin_type()))
        .cloned()
        .collect::<Vec<_>>();
    let download_client_runtime_plugins = runtime_plugins
        .iter()
        .filter(|plugin| plugin.descriptor.plugin_type() == "download_client")
        .cloned()
        .collect::<Vec<_>>();
    let subtitle_runtime_plugins = runtime_plugins
        .iter()
        .filter(|plugin| plugin.descriptor.plugin_type() == "subtitle_provider")
        .cloned()
        .collect::<Vec<_>>();
    let notification_runtime_plugins = runtime_plugins
        .iter()
        .filter(|plugin| plugin.descriptor.plugin_type() == "notification")
        .cloned()
        .collect::<Vec<_>>();
    let download_client_plugin_provider: Arc<dyn DownloadClientPluginProvider> =
        Arc::new(scryer_plugins::DynamicDownloadClientPluginProvider::new(
            scryer_plugins::build_download_client_plugin_provider_from_runtime_plugins(
                &download_client_runtime_plugins,
                &disabled_builtin_plugins,
            ),
        ));
    let fallback_download_client = Arc::new(NzbgetDownloadClient::with_staged_nzb_store(
        runtime_settings.nzbget_url,
        runtime_settings.nzbget_username,
        runtime_settings.nzbget_password,
        runtime_settings.nzbget_dupe_mode,
        staged_nzb_store.clone(),
        staged_nzb_pipeline_limit.clone(),
    ));
    let download_client = Arc::new(PrioritizedDownloadClientRouter::new(
        download_client_configs.clone(),
        settings_for_router.clone(),
        fallback_download_client,
        staged_nzb_store.clone(),
        staged_nzb_pipeline_limit.clone(),
        Some(download_client_plugin_provider.clone()),
    ));
    let indexer_stats: Arc<dyn scryer_application::IndexerStatsTracker> = Arc::new(
        scryer_infrastructure::InMemoryIndexerStatsTracker::new(Some(db.pool().clone())),
    );

    let dynamic_provider = scryer_plugins::DynamicPluginProvider::new(
        scryer_plugins::build_indexer_plugin_provider_from_runtime_plugins(
            &indexer_runtime_plugins,
            &disabled_builtin_plugins,
        ),
    );
    let plugin_provider: Arc<dyn IndexerPluginProvider> = Arc::new(dynamic_provider);
    let subtitle_plugin_provider: Arc<dyn SubtitlePluginProvider> =
        Arc::new(scryer_plugins::DynamicSubtitlePluginProvider::new(
            scryer_plugins::build_subtitle_plugin_provider_from_runtime_plugins(
                &subtitle_runtime_plugins,
                &disabled_builtin_plugins,
            ),
        ));

    let indexer_client = MultiIndexerSearchClient::new(
        indexer_configs.clone(),
        indexer_stats.clone(),
        plugin_provider.clone(),
    );

    let indexer_client = Arc::new(indexer_client);
    let title_image_processor = Arc::new(SqliteTitleImageProcessor::new());
    let title_images_for_route: Arc<dyn TitleImageRepository> =
        Arc::new(SqliteLibraryStateStore::new(&db));
    let metadata_gateway_url = std::env::var("SCRYER_METADATA_GATEWAY_GRAPHQL_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| SMG_GRAPHQL_URL.map(String::from))
        .unwrap_or_else(|| "http://127.0.0.1:8090/graphql".to_string());
    // TODO: Remove SCRYER_METADATA_GATEWAY_INSECURE once the gateway has proper TLS certificates.
    let metadata_gateway_insecure = std::env::var("SCRYER_METADATA_GATEWAY_INSECURE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let smg_registration_secret = SMG_REGISTRATION_SECRET
        .map(String::from)
        .or_else(|| std::env::var("SCRYER_SMG_REGISTRATION_SECRET").ok())
        .filter(|s| !s.is_empty());

    // Prefer an explicit managed JWT signing secret so restored bundles can
    // preserve instance identity across environments. Fall back to the SMG
    // registration secret or a persistent local secret for fresh installs.
    let jwt_signing_salt = match normalize_env_option("SCRYER_JWT_SIGNING_SECRET") {
        Some(secret) => secret,
        None => match &smg_registration_secret {
            Some(secret) => secret.clone(),
            None => {
                tracing::warn!(
                    "no SMG registration secret available; using persistent local JWT salt"
                );
                load_or_create_persistent_jwt_signing_salt(&data_dir)?
            }
        },
    };

    let smg_ca_cert = SMG_CA_CERT
        .map(String::from)
        .or_else(|| std::env::var("SCRYER_SMG_CA_CERT").ok())
        .filter(|s| !s.is_empty());
    let metadata_gateway = Arc::new(MetadataGatewayClient::new(
        metadata_gateway_url,
        metadata_gateway_insecure,
        db.clone(),
        SmgEnrollmentConfig {
            registration_secret: smg_registration_secret,
            ca_cert: smg_ca_cert,
        },
    ));
    let library_scanner = Arc::new(FileSystemLibraryScanner::new());
    let library_renamer = Arc::new(FileSystemLibraryRenamer::new());

    let (tracked_download_tx, tracked_download_rx) = tokio::sync::mpsc::channel(64);
    let library_state_store = Arc::new(SqliteLibraryStateStore::new(&db));

    // Warm up SMG enrollment so the mTLS client is ready before the first real
    // metadata query, and check for version incompatibility.
    let metadata_gateway_for_warmup = metadata_gateway.clone();
    tokio::spawn(async move {
        if let Some(incompat) = metadata_gateway_for_warmup.warm_enrollment().await {
            log_smg_version_incompatibility(&incompat);
        }
        if !metadata_gateway_for_warmup.compatibility_polling_enabled() {
            return;
        }

        let phase = loop {
            match metadata_gateway_for_warmup
                .version_compatibility_poll_phase()
                .await
            {
                Ok(phase) => break phase,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "failed to derive SMG version compatibility poll phase"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(15 * 60)).await;
                }
            }
        };

        let mut minimum_delay = MetadataGatewayClient::version_compatibility_startup_guard();
        loop {
            let delay =
                MetadataGatewayClient::next_version_compatibility_poll_delay(phase, minimum_delay);
            minimum_delay = std::time::Duration::from_secs(0);
            tokio::time::sleep(delay).await;

            match metadata_gateway_for_warmup
                .refresh_version_compatibility(true)
                .await
            {
                Ok(Some(incompat)) => log_smg_version_incompatibility(&incompat),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(error = %error, "SMG version compatibility refresh failed");
                }
            }
        }
    });

    let notif_provider = scryer_plugins::DynamicNotificationPluginProvider::new(
        scryer_plugins::build_notification_plugin_provider_from_runtime_plugins(
            &notification_runtime_plugins,
            &disabled_builtin_plugins,
        ),
    );
    let notification_store = Arc::new(SqliteNotificationStore::new(&db));
    let workflow_store = Arc::new(SqliteWorkflowStore::new(&db));
    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings_for_router.clone(),
        quality_profiles,
        db_path.clone(),
    )
    .with_libraries(libraries)
    .with_library_state_store(library_state_store)
    .with_customization_store(customization_store)
    .with_acquisition_state(workflow_store.clone())
    .with_domain_events(workflow_store.clone())
    .with_download_submissions(workflow_store.clone())
    .with_download_queue_commands(workflow_store.clone())
    .with_external_import_monitor_snapshots(workflow_store.clone())
    .with_import_artifacts(workflow_store.clone())
    .with_imports(workflow_store.clone())
    .with_job_runs(workflow_store.clone())
    .with_notification_store(notification_store)
    .with_system_info(settings_store.clone())
    .with_smg_registration_secret(
        SMG_REGISTRATION_SECRET
            .map(String::from)
            .or_else(|| std::env::var("SCRYER_SMG_REGISTRATION_SECRET").ok())
            .filter(|value| !value.is_empty()),
    )
    .with_smg_ca_cert(
        SMG_CA_CERT
            .map(String::from)
            .or_else(|| std::env::var("SCRYER_SMG_CA_CERT").ok())
            .filter(|value| !value.is_empty()),
    )
    .with_smg_gateway_url(Some(
        std::env::var("SCRYER_METADATA_GATEWAY_GRAPHQL_URL")
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| SMG_GRAPHQL_URL.map(String::from))
            .unwrap_or_else(|| "http://127.0.0.1:8090/graphql".to_string()),
    ))
    .with_metadata_gateway(metadata_gateway)
    .with_library_scanner(library_scanner)
    .with_library_renamer(library_renamer)
    .with_file_importer(Arc::new(scryer_infrastructure::FsFileImporter::new()))
    .with_title_image_processor(title_image_processor)
    .with_staged_nzb_store(staged_nzb_store)
    .with_staged_nzb_pipeline_limit(staged_nzb_pipeline_limit)
    .with_indexer_stats(indexer_stats)
    .with_plugin_provider(plugin_provider)
    .with_download_client_plugin_provider(download_client_plugin_provider.clone())
    .with_subtitle_provider_configs(subtitle_provider_configs)
    .with_subtitle_plugin_provider(subtitle_plugin_provider)
    .with_notification_provider(Arc::new(notif_provider))
    .with_workflow_operations(workflow_store)
    .with_tracked_download_handle(TrackedDownloadHandle::new(tracked_download_tx))
    .build();

    let app_use_case = AppUseCase::new(
        services,
        scryer_application::JwtAuthConfig {
            issuer: jwt_issuer,
            access_ttl_seconds: jwt_access_ttl_seconds as usize,
            jwt_signing_salt,
        },
        facet_registry,
    );

    app_use_case.connect_library_scan_tracker().await;

    if let Err(e) = app_use_case.reconcile_default_library_roots().await {
        tracing::warn!(error = %e, "failed to reconcile default library roots on startup");
    }

    if let Err(e) = app_use_case.refresh_plugin_catalog_internal().await {
        tracing::warn!(error = %e, "failed to refresh plugin catalog on startup");
    }

    if let Err(e) = app_use_case.migrate_user_entitlements().await {
        tracing::warn!(error = %e, "failed to migrate stored user entitlements on startup");
    }

    if let Err(e) = app_use_case.migrate_legacy_persona_preferences().await {
        tracing::warn!(error = %e, "failed to migrate legacy persona preferences on startup");
    }
    if let Err(e) = app_use_case
        .migrate_canonical_audio_persona_settings()
        .await
    {
        tracing::warn!(error = %e, "failed to migrate canonical audio/persona settings on startup");
    }
    if let Err(e) = app_use_case.rebuild_user_rules_engine().await {
        tracing::warn!(error = %e, "failed to rebuild user rules engine on startup");
    }
    if let Err(e) = app_use_case
        .migrate_legacy_opensubtitles_provider_config()
        .await
    {
        tracing::warn!(
            error = %e,
            "failed to migrate legacy opensubtitles settings into subtitle provider configs on startup"
        );
    }
    if let Err(e) = app_use_case.reconcile_indexer_configs().await {
        tracing::warn!(error = %e, "failed to reconcile indexer configs on startup");
    }
    if let Err(error) = seed_indexer_configs_from_env(&app_use_case).await {
        tracing::warn!(error = %error, "failed to seed indexer configs from environment");
    }
    if let Err(e) = app_use_case
        .ensure_indexer_routing_entries_for_existing_indexers()
        .await
    {
        tracing::warn!(error = %e, "failed to ensure indexer routing on startup");
    }
    if let Err(e) = app_use_case.normalize_routing_settings().await {
        tracing::warn!(error = %e, "failed to normalize routing settings on startup");
    }

    let saved_security_settings = app_use_case
        .security_settings()
        .await
        .map_err(|error| format!("failed to load security settings: {error}"))?;
    let auth_mode = resolve_auth_mode_from_env();
    let auth_runtime = AuthRuntimeStateHandle::new(AuthRuntimeStateSnapshot {
        form_login_enabled: saved_security_settings.form_login_enabled,
        skip_login_for_local_ips: saved_security_settings.skip_login_for_local_ips,
        effective_form_login_enabled: auth_mode
            .effective_form_login_enabled(saved_security_settings.form_login_enabled),
        env_override_active: auth_mode.env_override_active(),
        env_override_description: auth_mode.env_override_description.clone(),
        epoch: 0,
    });
    let log_buf_snapshot = log_ring_buffer.clone();
    let log_buf_subscribe = log_ring_buffer.clone();
    let schema = build_schema_with_log_buffer(
        app_use_case.clone(),
        auth_runtime.clone(),
        Some(LogBuffer::new(
            move |limit| log_buf_snapshot.snapshot(limit),
            move || log_buf_subscribe.subscribe(),
        )),
    );
    // Always run the download queue poller — it queries ALL enabled download
    // clients (NZBGet, SABnzbd, Weaver, plugins) and triggers imports for
    // completed downloads from any of them.
    tokio::spawn(start_download_queue_poller(
        app_use_case.clone(),
        shutdown_token.child_token(),
        tracked_download_rx,
    ));
    // Additionally start the Weaver WebSocket subscription bridge for
    // real-time UI updates (progress, state changes) when Weaver is
    // configured. The poller still handles import detection for all clients.
    if let Some((ws_url, api_key)) = resolve_weaver_ws_url(&app_use_case).await {
        tracing::info!(
            url = ws_url.as_str(),
            "using weaver subscription bridge for real-time download queue updates"
        );
        tokio::spawn(start_weaver_subscription_bridge(
            app_use_case.clone(),
            shutdown_token.child_token(),
            ws_url,
            api_key,
        ));
    }
    tokio::spawn(start_background_acquisition_poller(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_library_refresh_loop(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_title_hydration_loop(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_poster_loop(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_banner_loop(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_fanart_loop(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_notification_dispatcher(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_subtitle_poller(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_manual_import_poller(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_download_delete_poller(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    app_use_case.wake_title_image_loops();

    if auth_mode.used_legacy_dev_auto_login {
        tracing::warn!(
            "SCRYER_DEV_AUTO_LOGIN is deprecated; use SCRYER_AUTH_ENABLED=false instead"
        );
    }
    if auth_runtime.snapshot().effective_form_login_enabled {
        tracing::info!("running with authentication enabled");
        bootstrap_admin_password(&app_use_case).await;
    } else {
        let addr: SocketAddr = bind.parse().expect("invalid bind address");
        if !addr.ip().is_loopback() && !addr.ip().is_unspecified() {
            tracing::warn!(
                bind = %bind,
                "authentication is disabled on a non-loopback bind address; all requests will act as admin"
            );
        }
        tracing::warn!("running with authentication disabled; all requests act as admin");
    }

    let rate_limiter = ScryerRateLimiter::from_env();
    let auth_state = AuthState {
        app: app_use_case.clone(),
        schema: schema.clone(),
        auth_runtime: auth_runtime.clone(),
        rate_limiter: rate_limiter.clone(),
    };

    let cors_for_layer = cors.clone();
    let admin_migrations_db = settings_store.as_ref().clone();
    let admin_settings_db = settings_store.as_ref().clone();
    let admin_settings_app = app_use_case.clone();
    let admin_settings_auth_runtime = auth_runtime.clone();
    let backup_route_state = BackupRouteState {
        app: app_use_case.clone(),
        auth_runtime: auth_runtime.clone(),
        data_dir: data_dir.clone(),
        migration_mode,
    };
    let ws_auth_state = auth_state.clone();

    // WebSocket route must be outside CompressionLayer — compression wraps the
    // 101 upgrade response body and injects Content-Encoding, breaking the
    // WebSocket handshake.
    let ws_router = Router::new().route(
        "/graphql/ws",
        get(graphql_ws_handler).with_state(ws_auth_state),
    );

    let mut compressed_router = Router::new()
        .route("/health", get(health_handler))
        .route(
            "/graphql",
            post(graphql_handler).with_state(auth_state.clone()),
        )
        .route(
            "/images/titles/{title_id}/{kind}/{variant}",
            get(title_image_handler).with_state(title_images_for_route),
        )
        .route(
            "/admin/migrations",
            get(move || admin_migrations_handler(admin_migrations_db.clone())),
        )
        .route(
            "/admin/settings",
            get(
                move |headers: HeaderMap,
                      ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
                      Query(query): Query<AdminSettingsQuery>| {
                    admin_settings_list(
                        admin_settings_db.clone(),
                        admin_settings_app.clone(),
                        admin_settings_auth_runtime.clone(),
                        headers,
                        remote_addr,
                        query,
                    )
                },
            ),
        )
        .route(
            "/admin/backups/{filename}/download",
            get(download_backup_handler).with_state(backup_route_state.clone()),
        )
        .route(
            "/setup/restore/inspect",
            post(setup_restore_inspect_handler).with_state(backup_route_state.clone()),
        )
        .route(
            "/setup/restore/apply",
            post(setup_restore_apply_handler).with_state(backup_route_state),
        )
        .fallback(get(ui_fallback))
        .layer(axum::middleware::from_fn_with_state(
            auth_state.clone(),
            rate_limit_http_api,
        ))
        .layer(CompressionLayer::new().zstd(true).br(true).gzip(true));

    if let Some(ref handle) = metrics_handle {
        let h = handle.clone();
        compressed_router = compressed_router.route(
            "/metrics",
            get(move || {
                let h = h.clone();
                async move { h.render() }
            }),
        );
    }

    let app = ws_router
        .merge(compressed_router)
        .layer(axum::middleware::from_fn(move |request, next| {
            cors_handler(request, next, cors_for_layer.clone())
        }));

    match ui_asset_mode() {
        UiAssetMode::Filesystem(dist_dir) => {
            if Path::new(dist_dir).exists() {
                tracing::info!(path = %dist_dir.display(), "serving web UI from filesystem path");
            } else {
                tracing::warn!(
                    path = %dist_dir.display(),
                    "configured web UI path does not exist; serving fallback root notice"
                );
            }
        }
        UiAssetMode::Embedded => {
            tracing::info!("serving web UI from embedded assets bundled into this binary");
        }
        UiAssetMode::Fallback => {
            tracing::warn!("no web UI assets found; serving fallback root notice");
        }
    }

    Ok(app)
}

async fn title_image_handler(
    State(repository): State<Arc<dyn TitleImageRepository>>,
    headers: HeaderMap,
    AxumPath((title_id, kind, variant)): AxumPath<(String, String, String)>,
) -> Response {
    let Some(kind) = TitleImageKind::parse(&kind) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let blob = match repository
        .get_title_image_blob(&title_id, kind, &variant)
        .await
    {
        Ok(Some(blob)) => blob,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(
                error = %error,
                title_id = %title_id,
                kind = kind.as_str(),
                variant = %variant,
                "failed to serve title image"
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let quoted_etag = format!("\"{}\"", blob.etag);
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| if_none_match_matches(value, &quoted_etag, &blob.etag))
    {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        let headers = response.headers_mut();
        if let Ok(value) = HeaderValue::from_str(&quoted_etag) {
            headers.insert(header::ETAG, value);
        }
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
        return response;
    }

    let body_len = blob.bytes.len();
    let mut response = blob.bytes.into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&blob.content_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    if let Ok(value) = HeaderValue::from_str(&body_len.to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    if let Ok(value) = HeaderValue::from_str(&format!("\"{}\"", blob.etag)) {
        headers.insert(header::ETAG, value);
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    response
}

fn if_none_match_matches(raw_header: &str, quoted_etag: &str, bare_etag: &str) -> bool {
    raw_header.split(',').map(str::trim).any(|candidate| {
        candidate == "*"
            || candidate == quoted_etag
            || candidate == bare_etag
            || candidate
                .strip_prefix("W/")
                .is_some_and(|weak| weak == quoted_etag || weak == bare_etag)
    })
}

/// ValidateOnly mode: check for pending migrations and exit.
async fn run_validate_only(db_path: &str, migration_mode: MigrationMode) {
    match SqliteServices::new_with_mode(db_path, migration_mode).await {
        Ok(_) => {}
        Err(error) => {
            let message = error.to_string();
            if let Some(pending) = extract_pending_migration_ids(&message) {
                for migration_id in pending {
                    eprintln!("{migration_id}");
                }
            } else {
                eprintln!("{error}");
            }
            std::process::exit(1);
        }
    }
}

async fn shutdown_signal(token: CancellationToken) {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("received SIGINT, shutting down");
        }
        _ = terminate => {
            tracing::info!("received SIGTERM, shutting down");
        }
        _ = token.cancelled() => {}
    }
    token.cancel();

    // Hard exit if graceful shutdown takes too long.
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        tracing::warn!("graceful shutdown timed out, forcing exit");
        std::process::exit(0);
    });
}

/// Extract `--data-dir <path>` or `--data-dir=<path>` from the arg list,
/// removing those elements so the remaining args are clean for subcommand dispatch.
fn extract_data_dir(args: &mut Vec<String>) -> Option<PathBuf> {
    let mut i = 1; // skip binary name
    while i < args.len() {
        if args[i] == "--data-dir" {
            args.remove(i);
            if i < args.len() {
                return Some(PathBuf::from(args.remove(i)));
            }
            eprintln!("--data-dir requires a path argument");
            std::process::exit(1);
        } else if let Some(value) = args[i].strip_prefix("--data-dir=") {
            let path = PathBuf::from(value);
            args.remove(i);
            return Some(path);
        } else {
            i += 1;
        }
    }
    None
}

/// Resolve the data directory from CLI flag or platform default.
///
/// Priority: `--data-dir` flag > platform default via `directories` crate.
/// The env var `SCRYER_DB_PATH` can still override the *database path* specifically,
/// but the data directory itself is resolved here.
fn resolve_data_dir(cli_override: Option<&Path>) -> PathBuf {
    if let Some(dir) = cli_override {
        return dir.to_path_buf();
    }
    directories::ProjectDirs::from("", "", "scryer")
        .map(|p| p.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn load_env_file(data_dir: Option<&Path>, include_managed_instance_secrets: bool) {
    // Load in reverse priority order: dotenvy skips vars already set, so the
    // last file loaded has lowest priority.  Load the crate-local file first
    // (highest priority), then cwd .env, then data-dir .env (lowest priority).
    let candidates = ["crates/scryer/.env", ".env"];
    let mut loaded = false;
    for candidate in candidates {
        if Path::new(candidate).exists() {
            let _ = dotenvy::from_path(candidate);
            loaded = true;
        }
    }
    // Also load .env from the data directory (lowest priority).
    if let Some(dir) = data_dir {
        let env_path = dir.join(".env");
        if env_path.exists() {
            let _ = dotenvy::from_path(env_path);
            loaded = true;
        }
        if include_managed_instance_secrets {
            let secrets_path = dir.join("instance-secrets.env");
            if secrets_path.exists() {
                let _ = dotenvy::from_path_override(secrets_path);
                loaded = true;
            }
        }
    }
    if !loaded {
        let _ = dotenvy::dotenv();
    }
}

/// Open the user's default browser when running natively (not in Docker).
/// Controlled by `SCRYER_OPEN_BROWSER` env var: "false" disables, default is auto-detect.
fn maybe_open_browser(url: &str) {
    // Respect explicit opt-out.
    if let Ok(val) = std::env::var("SCRYER_OPEN_BROWSER")
        && matches!(
            val.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        )
    {
        return;
    }
    // Skip in containers (Docker sets /.dockerenv).
    if Path::new("/.dockerenv").exists() {
        return;
    }
    if let Err(err) = open::that(url) {
        tracing::debug!(error = %err, "could not open browser");
    }
}

pub(crate) fn normalize_env_option(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn load_or_create_persistent_jwt_signing_salt(data_dir: &Path) -> std::io::Result<String> {
    use ring::rand::{SecureRandom, SystemRandom};
    use std::io::Write;

    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join("jwt-signing-secret");
    match std::fs::read_to_string(&path) {
        Ok(existing) => {
            let existing = existing.trim();
            if !existing.is_empty() {
                return Ok(existing.to_string());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let rng = SystemRandom::new();
    let mut bytes = [0_u8; 32];
    rng.fill(&mut bytes)
        .map_err(|_| std::io::Error::other("failed to generate JWT signing secret"))?;
    let secret = format!("scryer-jwt-v1-{}", hex_bytes(&bytes));

    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            file.write_all(secret.as_bytes())?;
            file.write_all(b"\n")?;
            Ok(secret)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = std::fs::read_to_string(&path)?;
            let existing = existing.trim();
            if existing.is_empty() {
                Err(std::io::Error::other("JWT signing secret file is empty"))
            } else {
                Ok(existing.to_string())
            }
        }
        Err(error) => Err(error),
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

fn parse_env_bool_value(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn resolve_auth_mode(
    auth_enabled_raw: Option<&str>,
    legacy_dev_auto_login_raw: Option<&str>,
) -> AuthModeConfig {
    if let Some(auth_enabled) = auth_enabled_raw.and_then(parse_env_bool_value) {
        return AuthModeConfig {
            env_override_form_login_enabled: Some(auth_enabled),
            env_override_description: Some(format!("SCRYER_AUTH_ENABLED={auth_enabled}")),
            used_legacy_dev_auto_login: false,
        };
    }

    let used_legacy_dev_auto_login = matches!(
        legacy_dev_auto_login_raw.and_then(parse_env_bool_value),
        Some(true)
    );

    AuthModeConfig {
        env_override_form_login_enabled: used_legacy_dev_auto_login.then_some(false),
        env_override_description: used_legacy_dev_auto_login
            .then_some("SCRYER_DEV_AUTO_LOGIN=true".to_string()),
        used_legacy_dev_auto_login,
    }
}

fn resolve_auth_mode_from_env() -> AuthModeConfig {
    resolve_auth_mode(
        normalize_env_option("SCRYER_AUTH_ENABLED").as_deref(),
        normalize_env_option("SCRYER_DEV_AUTO_LOGIN").as_deref(),
    )
}

fn parse_env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

async fn check_version_upgrade(settings_store: SqliteSettingsStore) -> VersionLifecycle {
    const SCOPE: &str = "system";
    const KEY: &str = "last_run_version";

    let previous = settings_store
        .get_setting_with_defaults(SCOPE, KEY, None)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.value_json)
        .and_then(|v| serde_json::from_str::<String>(&v).ok());

    let lifecycle = match previous.as_deref() {
        Some(prev) if prev == VERSION => {
            tracing::debug!(version = VERSION, "version unchanged");
            VersionLifecycle::Unchanged
        }
        Some(prev) => {
            tracing::info!(
                previous_version = prev,
                current_version = VERSION,
                "upgraded from {prev} to {VERSION}"
            );
            VersionLifecycle::Upgraded
        }
        None => {
            tracing::info!(version = VERSION, "first run — recording version");
            VersionLifecycle::FirstRun
        }
    };

    let version_json = serde_json::to_string(VERSION).unwrap();
    if let Err(error) = settings_store
        .upsert_setting_value(SCOPE, KEY, None, version_json, "system", None)
        .await
    {
        tracing::warn!(error = %error, "failed to persist last_run_version");
    }

    lifecycle
}

async fn clear_legacy_history_retention_forever_override(settings_store: SqliteSettingsStore) {
    let keep_forever = settings_store
        .get_setting_with_defaults("system", HISTORY_KEEP_FOREVER_KEY, None)
        .await
        .ok()
        .flatten();
    let retention_days = settings_store
        .get_setting_with_defaults("system", HISTORY_RETENTION_DAYS_KEY, None)
        .await
        .ok()
        .flatten();

    let should_clear = keep_forever.as_ref().is_some_and(|record| {
        record.source.as_deref() == Some("migration")
            && record.value_json.as_deref() == Some("true")
            && !retention_days
                .as_ref()
                .is_some_and(scryer_infrastructure::SettingsValueRecord::has_override)
    });

    if !should_clear {
        return;
    }

    if let Err(error) = settings_store
        .delete_setting_value("system", HISTORY_KEEP_FOREVER_KEY, None)
        .await
    {
        tracing::warn!(
            error = %error,
            "failed to clear legacy history retention override"
        );
    } else {
        tracing::info!("cleared legacy history retention forever override");
    }
}

pub(crate) fn normalize_env_option_with_legacy<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    for name in names {
        if let Some(value) = normalize_env_option(name) {
            return Some(value);
        }
    }

    None
}

/// Check if the primary download client is weaver and return its WebSocket URL and API key.
async fn resolve_weaver_ws_url(app: &AppUseCase) -> Option<(String, Option<String>)> {
    let primary = app.primary_enabled_download_client_config().await.ok()??;

    if primary.client_type != "weaver" {
        return None;
    }

    let client = WeaverDownloadClient::from_config(&primary).ok()?;
    Some((client.ws_url(), client.api_key().map(str::to_string)))
}

fn runtime_normalized_constraint(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|constraint| !constraint.is_empty())
        .map(str::to_string)
}

fn runtime_installation_is_host_blocked(installation: &scryer_domain::PluginInstallation) -> bool {
    runtime_normalized_constraint(installation.scryer_constraint.as_deref()).is_some_and(
        |constraint| {
            scryer_plugins::host_version_matches_constraint(env!("CARGO_PKG_VERSION"), &constraint)
                .map(|matches| !matches)
                .unwrap_or(true)
        },
    )
}

fn runtime_installation_sdk_contract_is_host_compatible(
    installation: &scryer_domain::PluginInstallation,
) -> bool {
    match scryer_plugins::validate_sdk_contract(
        installation.plugin_id.as_str(),
        installation.sdk_version.as_str(),
        installation.sdk_constraint.as_str(),
        scryer_plugins::SDK_VERSION,
    ) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(
                plugin_id = installation.plugin_id.as_str(),
                version = installation.version.as_str(),
                sdk_version = installation.sdk_version.as_str(),
                sdk_constraint = installation.sdk_constraint.as_str(),
                error = %error,
                "skipping installed plugin with incompatible sdk contract"
            );
            false
        }
    }
}

async fn load_runtime_external_plugin_entry(
    installation: &scryer_domain::PluginInstallation,
    payload: scryer_domain::PersistedPluginWasmPayload,
) -> Option<RuntimePluginLoad> {
    match load_runtime_plugin_from_persisted_installation_payload(installation, &payload).await {
        Ok(runtime_plugin) => Some(runtime_plugin),
        Err(error) => {
            tracing::warn!(
                plugin_id = installation.plugin_id.as_str(),
                version = installation.version.as_str(),
                error = %error,
                "skipping installed plugin after persisted payload validation failed at startup"
            );
            None
        }
    }
}

async fn load_runtime_plugin_state(
    customization_store: &SqliteCustomizationStore,
) -> Result<(Vec<RuntimePluginLoad>, Vec<String>), String> {
    let enabled_plugins = customization_store
        .get_enabled_plugin_wasm_bytes()
        .await
        .map_err(|error| error.to_string())?;
    let mut runtime_plugins = Vec::new();
    let mut pending_plugins = enabled_plugins
        .into_iter()
        .filter_map(|(installation, payload)| {
            if !matches!(
                installation.source_kind,
                scryer_domain::PluginSourceKind::Downloaded
                    | scryer_domain::PluginSourceKind::Manual
            ) {
                return None;
            }
            if !runtime_installation_sdk_contract_is_host_compatible(&installation) {
                return None;
            }
            if runtime_installation_is_host_blocked(&installation) {
                return None;
            }

            payload.map(|payload| (installation, payload))
        });
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..RUNTIME_PLUGIN_LOAD_CONCURRENCY {
        let Some((installation, payload)) = pending_plugins.next() else {
            break;
        };
        tasks
            .spawn(async move { load_runtime_external_plugin_entry(&installation, payload).await });
    }
    while let Some(result) = tasks.join_next().await {
        let loaded =
            result.map_err(|error| format!("startup plugin load task panicked: {error}"))?;
        if let Some(entry) = loaded {
            runtime_plugins.push(entry);
        }
        if let Some((installation, payload)) = pending_plugins.next() {
            tasks.spawn(
                async move { load_runtime_external_plugin_entry(&installation, payload).await },
            );
        }
    }

    let disabled_builtin_plugins = customization_store
        .list_plugin_installations()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|installation| installation.is_builtin && !installation.is_enabled)
        .map(|installation| installation.provider_type)
        .collect::<Vec<_>>();

    Ok((runtime_plugins, disabled_builtin_plugins))
}

async fn bootstrap_plugin_installations(
    customization_store: &SqliteCustomizationStore,
) -> Result<(), String> {
    let removed = customization_store
        .delete_incompatible_external_plugin_installations()
        .await
        .map_err(|error| error.to_string())?;
    for plugin_id in removed {
        tracing::warn!(
            plugin_id = plugin_id.as_str(),
            "removed incompatible legacy external plugin installation during startup bootstrap"
        );
    }

    seed_builtin_plugin_installations(customization_store).await
}

async fn seed_builtin_plugin_installations(
    customization_store: &SqliteCustomizationStore,
) -> Result<(), String> {
    struct BuiltinPluginSeed {
        name: String,
        description: String,
        version: String,
        sdk_version: String,
        sdk_constraint: String,
        plugin_type: String,
        provider_type: String,
    }

    let mut builtins = Vec::new();

    let indexer_provider = scryer_plugins::build_indexer_plugin_provider(&[], &[]);
    for provider_type in indexer_provider.builtin_provider_types() {
        let provider_key = provider_type.trim().to_ascii_lowercase();
        let Some(name) = indexer_provider.plugin_name_for_provider(&provider_key) else {
            continue;
        };
        let description = indexer_provider
            .plugin_description_for_provider(&provider_key)
            .unwrap_or_default();
        let Some(version) = indexer_provider.plugin_version_for_provider(&provider_key) else {
            continue;
        };
        let Some(sdk_version) = indexer_provider.plugin_sdk_version_for_provider(&provider_key)
        else {
            continue;
        };
        let Some(sdk_constraint) =
            indexer_provider.plugin_sdk_constraint_for_provider(&provider_key)
        else {
            continue;
        };
        let plugin_type = indexer_provider
            .plugin_type_for_provider(&provider_key)
            .unwrap_or_else(|| "indexer".to_string());
        builtins.push(BuiltinPluginSeed {
            name,
            description,
            version,
            sdk_version,
            sdk_constraint,
            plugin_type,
            provider_type: provider_key,
        });
    }

    let subtitle_provider = scryer_plugins::build_subtitle_plugin_provider(&[], &[]);
    for provider_type in subtitle_provider.builtin_provider_types() {
        let provider_key = provider_type.trim().to_ascii_lowercase();
        let Some(name) = subtitle_provider.plugin_name_for_provider(&provider_key) else {
            continue;
        };
        let description = subtitle_provider
            .plugin_description_for_provider(&provider_key)
            .unwrap_or_default();
        let Some(version) = subtitle_provider.plugin_version_for_provider(&provider_key) else {
            continue;
        };
        let Some(sdk_version) = subtitle_provider.plugin_sdk_version_for_provider(&provider_key)
        else {
            continue;
        };
        let Some(sdk_constraint) =
            subtitle_provider.plugin_sdk_constraint_for_provider(&provider_key)
        else {
            continue;
        };
        builtins.push(BuiltinPluginSeed {
            name,
            description,
            version,
            sdk_version,
            sdk_constraint,
            plugin_type: "subtitle_provider".to_string(),
            provider_type: provider_key,
        });
    }

    let download_client_provider = scryer_plugins::build_download_client_plugin_provider(&[], &[]);
    for provider_type in download_client_provider.builtin_provider_types() {
        let provider_key = provider_type.trim().to_ascii_lowercase();
        let Some(name) = download_client_provider.plugin_name_for_provider(&provider_key) else {
            continue;
        };
        let description = download_client_provider
            .plugin_description_for_provider(&provider_key)
            .unwrap_or_default();
        let Some(version) = download_client_provider.plugin_version_for_provider(&provider_key)
        else {
            continue;
        };
        let Some(sdk_version) =
            download_client_provider.plugin_sdk_version_for_provider(&provider_key)
        else {
            continue;
        };
        let Some(sdk_constraint) =
            download_client_provider.plugin_sdk_constraint_for_provider(&provider_key)
        else {
            continue;
        };
        builtins.push(BuiltinPluginSeed {
            name,
            description,
            version,
            sdk_version,
            sdk_constraint,
            plugin_type: "download_client".to_string(),
            provider_type: provider_key,
        });
    }

    let notification_provider = scryer_plugins::build_notification_plugin_provider(&[], &[]);
    for provider_type in notification_provider.builtin_provider_types() {
        let provider_key = provider_type.trim().to_ascii_lowercase();
        let Some(name) = notification_provider.plugin_name_for_provider(&provider_key) else {
            continue;
        };
        let description = notification_provider
            .plugin_description_for_provider(&provider_key)
            .unwrap_or_default();
        let Some(version) = notification_provider.plugin_version_for_provider(&provider_key) else {
            continue;
        };
        let Some(sdk_version) =
            notification_provider.plugin_sdk_version_for_provider(&provider_key)
        else {
            continue;
        };
        let Some(sdk_constraint) =
            notification_provider.plugin_sdk_constraint_for_provider(&provider_key)
        else {
            continue;
        };
        builtins.push(BuiltinPluginSeed {
            name,
            description,
            version,
            sdk_version,
            sdk_constraint,
            plugin_type: "notification".to_string(),
            provider_type: provider_key,
        });
    }

    let builtin_lookup_key = |plugin_type: &str, provider_type: &str| {
        let family = match plugin_type {
            "indexer" | "usenet_indexer" | "torrent_indexer" => "indexer",
            other => other,
        };
        format!("{family}::{}", provider_type.trim().to_ascii_lowercase())
    };

    let builtin_keys = builtins
        .iter()
        .map(|builtin| builtin_lookup_key(&builtin.plugin_type, &builtin.provider_type))
        .collect::<std::collections::HashSet<_>>();

    for builtin in builtins {
        customization_store
            .seed_builtin(
                &builtin.provider_type,
                &builtin.name,
                &builtin.description,
                &builtin.version,
                &builtin.sdk_version,
                &builtin.sdk_constraint,
                &builtin.plugin_type,
                &builtin.provider_type,
            )
            .await
            .map_err(|error| error.to_string())?;
    }

    let stale_builtin_plugin_ids = customization_store
        .list_plugin_installations()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|installation| {
            installation.is_builtin
                && !builtin_keys.contains(&builtin_lookup_key(
                    &installation.plugin_type,
                    &installation.provider_type,
                ))
        })
        .map(|installation| installation.plugin_id)
        .collect::<Vec<_>>();

    for plugin_id in stale_builtin_plugin_ids {
        customization_store
            .delete_plugin_installation(&plugin_id)
            .await
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AuthModeConfig, VersionLifecycle, bootstrap_plugin_installations, check_version_upgrade,
        clear_legacy_history_retention_forever_override, load_runtime_plugin_state,
        resolve_auth_mode, seed_service_setting_definitions, title_image_handler,
    };
    use chrono::Utc;
    use std::sync::Arc;

    use crate::base_path::{BasePath, mount_router};
    use crate::{
        HISTORY_KEEP_FOREVER_KEY, HISTORY_RETENTION_DAYS_KEY,
        settings_bootstrap::SETTINGS_SCOPE_SYSTEM,
    };
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use axum::routing::get;
    use scryer_application::{
        AppResult, PluginInstallationRepository, TitleImageBlob, TitleImageKind,
        TitleImageReplacement, TitleImageRepository, TitleImageSyncTask,
    };
    use scryer_infrastructure::{
        MigrationMode, SqliteCustomizationStore, SqliteServices, SqliteSettingsStore,
    };
    use tempfile::tempdir;
    use tower::ServiceExt;

    #[derive(Default)]
    struct MockTitleImageRepository {
        blob: Option<TitleImageBlob>,
    }

    #[async_graphql::async_trait::async_trait]
    impl TitleImageRepository for MockTitleImageRepository {
        async fn list_titles_requiring_image_refresh(
            &self,
            _kind: TitleImageKind,
            _limit: usize,
        ) -> AppResult<Vec<TitleImageSyncTask>> {
            Ok(Vec::new())
        }

        async fn replace_title_image(
            &self,
            _title_id: &str,
            _replacement: TitleImageReplacement,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn replace_title_image_and_append_event(
            &self,
            _title_id: &str,
            _replacement: TitleImageReplacement,
            event: scryer_domain::NewDomainEvent,
        ) -> AppResult<scryer_domain::DomainEvent> {
            Ok(scryer_domain::DomainEvent {
                sequence: 1,
                event_id: event.event_id,
                occurred_at: event.occurred_at,
                actor_user_id: event.actor_user_id,
                title_id: event.title_id,
                facet: event.facet,
                correlation_id: event.correlation_id,
                causation_id: event.causation_id,
                schema_version: event.schema_version,
                stream: event.stream,
                payload: event.payload,
            })
        }

        async fn get_title_image_blob(
            &self,
            _title_id: &str,
            _kind: TitleImageKind,
            _variant_key: &str,
        ) -> AppResult<Option<TitleImageBlob>> {
            Ok(self.blob.clone())
        }
    }

    #[test]
    fn auth_defaults_to_disabled() {
        assert_eq!(
            resolve_auth_mode(None, None),
            AuthModeConfig {
                env_override_form_login_enabled: None,
                env_override_description: None,
                used_legacy_dev_auto_login: false,
            }
        );
    }

    #[test]
    fn explicit_auth_enabled_wins() {
        assert_eq!(
            resolve_auth_mode(Some("true"), Some("true")),
            AuthModeConfig {
                env_override_form_login_enabled: Some(true),
                env_override_description: Some("SCRYER_AUTH_ENABLED=true".to_string()),
                used_legacy_dev_auto_login: false,
            }
        );
    }

    #[tokio::test]
    async fn runtime_plugin_state_succeeds_after_bootstrap_deletes_legacy_external_rows() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("plugins.db");
        let services = SqliteServices::new(db_path.to_string_lossy())
            .await
            .unwrap();
        let customization = SqliteCustomizationStore::new(&services);
        let now = Utc::now();

        customization
            .create_plugin_installation(
                &scryer_domain::PluginInstallation {
                    id: scryer_domain::Id::new().0,
                    plugin_id: "legacy".to_string(),
                    name: "Legacy".to_string(),
                    description: "legacy plugin".to_string(),
                    version: "0.1.0".to_string(),
                    sdk_version: "1.3.0".to_string(),
                    sdk_constraint: ">=1.3.0, <1.4.0".to_string(),
                    scryer_constraint: None,
                    plugin_type: "notification".to_string(),
                    provider_type: "legacy".to_string(),
                    source_kind: scryer_domain::PluginSourceKind::Downloaded,
                    is_enabled: true,
                    is_builtin: false,
                    wasm_encoding: scryer_domain::PluginWasmEncoding::Identity,
                    wasm_digest_algo: None,
                    source_url: Some("https://example.com/legacy.wasm".to_string()),
                    support_tier: scryer_domain::PluginSupportTier::Official,
                    publisher: None,
                    docs_url: None,
                    source_repo: None,
                    manifest_url: None,
                    wasm_digest: None,
                    artifact_digest: None,
                    descriptor_json: None,
                    installed_at: now,
                    updated_at: now,
                },
                Some(&[1_u8, 2, 3]),
            )
            .await
            .expect("seed legacy plugin row");

        bootstrap_plugin_installations(&customization)
            .await
            .expect("bootstrap plugin installations");

        let (runtime_plugins, disabled_builtins) = load_runtime_plugin_state(&customization)
            .await
            .expect("load runtime plugin state");

        assert!(runtime_plugins.is_empty());
        assert!(disabled_builtins.is_empty());
        assert!(
            customization
                .get_plugin_installation("legacy")
                .await
                .expect("read plugin installation")
                .is_none()
        );
    }

    #[tokio::test]
    async fn runtime_plugin_state_skips_corrupted_external_plugin_rows() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("plugins.db");
        let services = SqliteServices::new(db_path.to_string_lossy())
            .await
            .unwrap();
        let customization = SqliteCustomizationStore::new(&services);
        let now = Utc::now();
        let compressed = vec![
            0x28, 0xb5, 0x2f, 0xfd, 0x24, 0x00, 0x01, 0x00, 0x00, 0x99, 0xe9, 0xd8, 0x51,
        ];

        customization
            .create_plugin_installation(
                &scryer_domain::PluginInstallation {
                    id: scryer_domain::Id::new().0,
                    plugin_id: "corrupt".to_string(),
                    name: "Corrupt".to_string(),
                    description: "corrupt plugin".to_string(),
                    version: "0.1.0".to_string(),
                    sdk_version: scryer_plugins::SDK_VERSION.to_string(),
                    sdk_constraint: scryer_plugins::sdk_constraint_or_legacy(
                        scryer_plugins::SDK_VERSION,
                        "",
                    ),
                    scryer_constraint: None,
                    plugin_type: "notification".to_string(),
                    provider_type: "corrupt".to_string(),
                    source_kind: scryer_domain::PluginSourceKind::Downloaded,
                    is_enabled: true,
                    is_builtin: false,
                    wasm_encoding: scryer_domain::PluginWasmEncoding::Zstd,
                    wasm_digest_algo: Some("blake3".to_string()),
                    source_url: Some("https://example.com/corrupt.wasm.zst".to_string()),
                    support_tier: scryer_domain::PluginSupportTier::Official,
                    publisher: None,
                    docs_url: None,
                    source_repo: None,
                    manifest_url: None,
                    wasm_digest: Some("deadbeef".to_string()),
                    artifact_digest: Some("blake3:abcd".to_string()),
                    descriptor_json: Some(
                        r#"{"id":"corrupt","name":"Corrupt","version":"0.1.0","sdk_version":"1.5.0","sdk_constraint":">=1.5.0, <2.0.0","socket_permissions":[],"provider":{"kind":"notification","provider_type":"corrupt","provider_aliases":[],"config_fields":[],"allowed_hosts":[],"default_base_url":null,"capabilities":{"supported_events":[]}}}"#.to_string(),
                    ),
                    installed_at: now,
                    updated_at: now,
                },
                Some(compressed.as_slice()),
            )
            .await
            .expect("seed corrupt plugin row");

        bootstrap_plugin_installations(&customization)
            .await
            .expect("bootstrap plugin installations");

        let (runtime_plugins, disabled_builtins) = load_runtime_plugin_state(&customization)
            .await
            .expect("load runtime plugin state");

        assert!(runtime_plugins.is_empty());
        assert!(disabled_builtins.is_empty());
        assert!(
            customization
                .get_plugin_installation("corrupt")
                .await
                .expect("read plugin installation")
                .is_some()
        );
    }

    #[test]
    fn explicit_auth_disabled_wins_over_legacy_alias() {
        assert_eq!(
            resolve_auth_mode(Some("false"), Some("true")),
            AuthModeConfig {
                env_override_form_login_enabled: Some(false),
                env_override_description: Some("SCRYER_AUTH_ENABLED=false".to_string()),
                used_legacy_dev_auto_login: false,
            }
        );
    }

    #[test]
    fn legacy_dev_auto_login_disables_auth_when_new_flag_absent() {
        assert_eq!(
            resolve_auth_mode(None, Some("true")),
            AuthModeConfig {
                env_override_form_login_enabled: Some(false),
                env_override_description: Some("SCRYER_DEV_AUTO_LOGIN=true".to_string()),
                used_legacy_dev_auto_login: true,
            }
        );
    }

    #[test]
    fn invalid_auth_flag_falls_back_to_default_disabled() {
        assert_eq!(
            resolve_auth_mode(Some("garbage"), None),
            AuthModeConfig {
                env_override_form_login_enabled: None,
                env_override_description: None,
                used_legacy_dev_auto_login: false,
            }
        );
    }

    #[tokio::test]
    async fn title_image_route_serves_cached_bytes_with_headers() {
        let repo: Arc<dyn TitleImageRepository> = Arc::new(MockTitleImageRepository {
            blob: Some(TitleImageBlob {
                content_type: "image/avif".to_string(),
                etag: "abc123".to_string(),
                bytes: vec![1, 2, 3, 4],
            }),
        });
        let app = Router::new().route(
            "/images/titles/{title_id}/{kind}/{variant}",
            get(title_image_handler).with_state(repo),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/images/titles/title-1/poster/w500")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/avif"
        );
        assert_eq!(response.headers().get(header::ETAG).unwrap(), "\"abc123\"");
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
    }

    #[tokio::test]
    async fn title_image_route_returns_not_found_for_missing_images() {
        let repo: Arc<dyn TitleImageRepository> = Arc::new(MockTitleImageRepository::default());
        let app = Router::new().route(
            "/images/titles/{title_id}/{kind}/{variant}",
            get(title_image_handler).with_state(repo),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/images/titles/title-1/poster/w500")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn title_image_route_returns_not_modified_for_matching_etag() {
        let repo: Arc<dyn TitleImageRepository> = Arc::new(MockTitleImageRepository {
            blob: Some(TitleImageBlob {
                content_type: "image/avif".to_string(),
                etag: "abc123".to_string(),
                bytes: vec![1, 2, 3, 4],
            }),
        });
        let app = Router::new().route(
            "/images/titles/{title_id}/{kind}/{variant}",
            get(title_image_handler).with_state(repo),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/images/titles/title-1/poster/w500")
                    .header(header::IF_NONE_MATCH, "\"abc123\"")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(response.headers().get(header::ETAG).unwrap(), "\"abc123\"");
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
    }

    #[tokio::test]
    async fn title_image_route_serves_under_prefixed_base_path() {
        let repo: Arc<dyn TitleImageRepository> = Arc::new(MockTitleImageRepository {
            blob: Some(TitleImageBlob {
                content_type: "image/avif".to_string(),
                etag: "abc123".to_string(),
                bytes: vec![1, 2, 3, 4],
            }),
        });
        let app = mount_router(
            Router::new().route(
                "/images/titles/{title_id}/{kind}/{variant}",
                get(title_image_handler).with_state(repo),
            ),
            &BasePath::from_raw(Some("/scryer/")),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/scryer/images/titles/title-1/poster/w500")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    async fn bootstrap_settings_store() -> (tempfile::TempDir, SqliteSettingsStore) {
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join("scryer.db");
        let services = SqliteServices::new_with_mode(
            db_path.to_string_lossy().to_string(),
            MigrationMode::Apply,
        )
        .await
        .expect("sqlite services");
        let store = SqliteSettingsStore::new(&services);
        seed_service_setting_definitions(store.clone())
            .await
            .expect("seed setting definitions");
        (temp, store)
    }

    #[tokio::test]
    async fn legacy_migration_history_override_is_cleared_back_to_default() {
        let (_temp, store) = bootstrap_settings_store().await;
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                "last_run_version",
                None,
                "\"0.10.0\"",
                "test",
                None,
            )
            .await
            .expect("seed previous version");

        let lifecycle = check_version_upgrade(store.clone()).await;
        assert_eq!(lifecycle, VersionLifecycle::Upgraded);

        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                HISTORY_KEEP_FOREVER_KEY,
                None,
                "true",
                "migration",
                None,
            )
            .await
            .expect("seed legacy migration override");

        clear_legacy_history_retention_forever_override(store.clone()).await;

        let keep_forever = store
            .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, HISTORY_KEEP_FOREVER_KEY, None)
            .await
            .expect("load keep forever")
            .expect("setting exists");
        assert_eq!(keep_forever.effective_value_json, "false");
        assert_eq!(keep_forever.value_json, None);
        assert_eq!(keep_forever.source, None);
    }

    #[tokio::test]
    async fn legacy_migration_history_override_is_preserved_when_user_has_retention_override() {
        let (_temp, store) = bootstrap_settings_store().await;
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                "last_run_version",
                None,
                "\"0.10.0\"",
                "test",
                None,
            )
            .await
            .expect("seed previous version");
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                HISTORY_RETENTION_DAYS_KEY,
                None,
                "30",
                "ui",
                Some("user-1".to_string()),
            )
            .await
            .expect("seed explicit retention override");
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                HISTORY_KEEP_FOREVER_KEY,
                None,
                "true",
                "migration",
                None,
            )
            .await
            .expect("seed legacy migration override");

        let lifecycle = check_version_upgrade(store.clone()).await;
        assert_eq!(lifecycle, VersionLifecycle::Upgraded);

        clear_legacy_history_retention_forever_override(store.clone()).await;

        let keep_forever = store
            .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, HISTORY_KEEP_FOREVER_KEY, None)
            .await
            .expect("load keep forever")
            .expect("setting exists");
        assert_eq!(keep_forever.effective_value_json, "true");
        assert_eq!(keep_forever.value_json.as_deref(), Some("true"));
        assert_eq!(keep_forever.source.as_deref(), Some("migration"));
    }
}
