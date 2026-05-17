#![allow(dead_code)]

use std::sync::Arc;

use async_graphql_axum::GraphQLRequest;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::json;
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use scryer_application::{
    AppServices, AppUseCase, FacetRegistry, IndexerPluginProvider, JwtAuthConfig,
    MovieFacetHandler, SeriesFacetHandler,
};
use scryer_infrastructure::sqlite::{
    LibraryStore, PluginStore, PostProcessingScriptStore, QualityProfileStore, RuleSetStore,
    SettingsStore, ShowStore, TitleStore, UserStore,
};
use scryer_infrastructure::{
    AcquisitionStore, DomainEventStore, DownloadClientConfigStore, DownloadQueueCommandStore,
    DownloadSubmissionStore, ExternalImportMonitorStore, FileSystemLibraryScanner,
    FileSystemStagedNzbStore, ImportStore, IndexerConfigStore, LibraryProbeStore,
    LibraryStateStore, MetadataGatewayClient, MultiIndexerSearchClient, NzbgetDownloadClient,
    ReleaseStore, SmgEnrollmentConfig, SqliteServices, TitleImageStore, WorkflowOperationStore,
};
use scryer_interface::context::{AuthRuntimeStateHandle, AuthRuntimeStateSnapshot};
use scryer_interface::{ApiSchema, build_schema};

/// Shared integration-test context.
///
/// Boots wiremock servers for external APIs, in-memory SQLite, real
/// infrastructure clients pointed at wiremock, a full `AppUseCase`,
/// GraphQL schema, and an axum server on a random port.
pub struct TestContext {
    pub nzbget_server: MockServer,
    pub nzbgeek_server: MockServer,
    pub smg_server: MockServer,
    /// Base URL of the test axum server (e.g. `http://127.0.0.1:12345`).
    pub app_url: String,
    pub schema: ApiSchema,
    pub auth_runtime: AuthRuntimeStateHandle,
    pub app: AppUseCase,
    pub titles: TitleStore,
    pub shows: ShowStore,
    pub libraries: LibraryStore,
    pub users: UserStore,
    pub customization: PluginStore,
    pub library_probe: LibraryProbeStore,
    pub library_state: LibraryStateStore,
    pub db: SqliteServices,
    pub settings_store: Arc<SettingsStore>,
    pub staged_nzb_store: Arc<FileSystemStagedNzbStore>,
    pub staged_nzb_dir: tempfile::TempDir,
}

pub fn disabled_auth_runtime_handle() -> AuthRuntimeStateHandle {
    AuthRuntimeStateHandle::new(AuthRuntimeStateSnapshot {
        form_login_enabled: false,
        skip_login_for_local_ips: false,
        effective_form_login_enabled: false,
        env_override_active: false,
        env_override_description: None,
        epoch: 0,
    })
}

impl TestContext {
    pub async fn new() -> Self {
        // Start wiremock mock servers for each external API
        let nzbget_server = MockServer::start().await;
        let nzbgeek_server = MockServer::start().await;
        let smg_server = MockServer::start().await;
        mount_default_smg_metadata_mocks(&smg_server).await;

        // In-memory SQLite with migrations applied
        let db = SqliteServices::new(":memory:")
            .await
            .expect("failed to create in-memory SQLite");
        let staged_nzb_dir = tempfile::TempDir::new().expect("failed to create staged nzb tempdir");
        let staged_nzb_store = Arc::new(
            FileSystemStagedNzbStore::new(staged_nzb_dir.path())
                .await
                .expect("failed to create staged nzb store"),
        );
        let staged_nzb_pipeline_limit = Arc::new(tokio::sync::Semaphore::new(4));
        let release_store = Arc::new(ReleaseStore::from_sqlite_services(&db));
        let settings_store = Arc::new(SettingsStore::from_sqlite_services(&db));
        let quality_profile_store = Arc::new(QualityProfileStore::from_sqlite_services(&db));

        // Real clients pointed at wiremock URLs
        let nzbget = NzbgetDownloadClient::with_staged_nzb_store(
            nzbget_server.uri(),
            Some("test-user".to_string()),
            Some("test-pass".to_string()),
            "SCORE".to_string(),
            staged_nzb_store.clone(),
            staged_nzb_pipeline_limit.clone(),
        );

        let indexer_config_store = Arc::new(IndexerConfigStore::from_sqlite_services(&db));
        let download_client_config_store =
            Arc::new(DownloadClientConfigStore::from_sqlite_services(&db));

        // Build indexer client backed by built-in WASM plugins (using DynamicPluginProvider
        // so reload_plugins works in integration tests)
        let plugin_provider: Arc<dyn IndexerPluginProvider> =
            Arc::new(scryer_plugins::DynamicPluginProvider::new(
                scryer_plugins::WasmIndexerPluginProvider::empty()
                    .with_builtin_asset(scryer_plugins::builtins::NZBGEEK)
                    .with_builtin_asset(scryer_plugins::builtins::NEWZNAB),
            ));
        let indexer_stats: Arc<dyn scryer_application::IndexerStatsTracker> = Arc::new(
            scryer_infrastructure::InMemoryIndexerStatsTracker::new(None),
        );
        let indexer_client = MultiIndexerSearchClient::new(
            indexer_config_store.clone(),
            indexer_stats.clone(),
            plugin_provider.clone(),
        );

        let metadata_gateway = MetadataGatewayClient::new(
            format!("{}/graphql", smg_server.uri()),
            true, // accept invalid certs (wiremock is plain HTTP)
            db.clone(),
            SmgEnrollmentConfig {
                registration_secret: None,
                ca_cert: None,
            },
        );

        // Build repository implementations from the shared DB runtime.
        let title_store = TitleStore::sqlite(&db);
        let show_store = ShowStore::sqlite(&db);
        let library_store = LibraryStore::sqlite(&db);
        let user_store = UserStore::sqlite(&db);
        let titles: Arc<dyn scryer_application::TitleRepository> = Arc::new(title_store.clone());
        let shows: Arc<dyn scryer_application::ShowRepository> = Arc::new(show_store.clone());
        let users: Arc<dyn scryer_application::UserRepository> = Arc::new(user_store.clone());
        let indexer_configs: Arc<dyn scryer_application::IndexerConfigRepository> =
            indexer_config_store;
        let download_client_configs: Arc<dyn scryer_application::DownloadClientConfigRepository> =
            download_client_config_store;
        let release_attempts: Arc<dyn scryer_application::ReleaseAttemptRepository> = release_store;
        let settings: Arc<dyn scryer_application::SettingsRepository> = settings_store.clone();
        let quality_profiles: Arc<dyn scryer_application::QualityProfileRepository> =
            quality_profile_store.clone();

        let library_probe_store = LibraryProbeStore::from_sqlite_services(&db);
        let library_state_store = LibraryStateStore::from_sqlite_services(&db);
        let title_image_store = TitleImageStore::from_sqlite_services(&db);
        let rule_set_store = RuleSetStore::from_sqlite_services(&db);
        let post_processing_script_store = PostProcessingScriptStore::from_sqlite_services(&db);
        let plugin_store = PluginStore::from_sqlite_services(&db);
        let domain_event_store = Arc::new(DomainEventStore::from_sqlite_services(&db));
        let acquisition_store = Arc::new(AcquisitionStore::from_sqlite_services(&db));
        let download_submission_store =
            Arc::new(DownloadSubmissionStore::from_sqlite_services(&db));
        let import_store = Arc::new(ImportStore::from_sqlite_services(&db));
        let external_import_monitor_store =
            Arc::new(ExternalImportMonitorStore::from_sqlite_services(&db));
        let download_queue_command_store =
            Arc::new(DownloadQueueCommandStore::from_sqlite_services(&db));
        let workflow_operation_store = Arc::new(WorkflowOperationStore::from_sqlite_services(&db));
        let services = AppServices::builder(
            titles,
            shows,
            users,
            indexer_configs,
            Arc::new(indexer_client),
            Arc::new(nzbget),
            download_client_configs,
            release_attempts,
            settings,
            quality_profiles,
            ":memory:".to_string(),
        )
        .with_media_files(Arc::new(library_state_store.clone()))
        .with_wanted_items(Arc::new(library_state_store.clone()))
        .with_pending_releases(Arc::new(library_state_store.clone()))
        .with_blocklist_repo(Arc::new(library_state_store.clone()))
        .with_library_probe_signatures(Arc::new(library_probe_store.clone()))
        .with_library_scan_unmatched_items(Arc::new(library_state_store.clone()))
        .with_title_images(Arc::new(title_image_store))
        .with_housekeeping(Arc::new(library_state_store.clone()))
        .with_subtitle_downloads(Arc::new(library_state_store.clone()))
        .with_libraries(Arc::new(library_store.clone()))
        .with_rule_set_store(Arc::new(rule_set_store))
        .with_post_processing_script_store(Arc::new(post_processing_script_store))
        .with_plugin_installation_store(Arc::new(plugin_store.clone()))
        .with_acquisition_state(acquisition_store)
        .with_domain_events(domain_event_store)
        .with_download_queue_commands(download_queue_command_store)
        .with_download_submissions(download_submission_store)
        .with_external_import_monitor_snapshots(external_import_monitor_store)
        .with_import_artifacts(import_store.clone())
        .with_imports(import_store)
        .with_job_runs(workflow_operation_store.clone())
        .with_system_info(settings_store.clone())
        .with_metadata_gateway(Arc::new(metadata_gateway))
        .with_library_scanner(Arc::new(FileSystemLibraryScanner::new()))
        .with_indexer_stats(indexer_stats)
        .with_plugin_provider(plugin_provider)
        .with_staged_nzb_store(staged_nzb_store.clone())
        .with_staged_nzb_pipeline_limit(staged_nzb_pipeline_limit)
        .with_workflow_operations(workflow_operation_store)
        .build();

        // Facet registry with all built-in facets
        let mut registry = FacetRegistry::new();
        registry.register(Arc::new(MovieFacetHandler));
        registry.register(Arc::new(SeriesFacetHandler::new(
            scryer_domain::MediaFacet::Series,
        )));
        registry.register(Arc::new(SeriesFacetHandler::new(
            scryer_domain::MediaFacet::Anime,
        )));
        let facet_registry = Arc::new(registry);

        let app = AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "scryer-test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            facet_registry,
        );

        // Build the GraphQL schema with authentication disabled.
        let auth_runtime = disabled_auth_runtime_handle();
        let schema = build_schema(app.clone(), auth_runtime.clone());

        // Start axum server on a random port
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind test server");
        let addr = listener.local_addr().expect("failed to get local addr");
        let app_url = format!("http://{addr}");

        let router = build_test_router(app.clone(), schema.clone(), auth_runtime.clone());
        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("test server failed");
        });

        Self {
            nzbget_server,
            nzbgeek_server,
            smg_server,
            app_url,
            schema,
            auth_runtime,
            app,
            titles: title_store,
            shows: show_store,
            libraries: library_store,
            users: user_store,
            customization: plugin_store,
            library_probe: library_probe_store,
            library_state: library_state_store,
            db,
            settings_store,
            staged_nzb_store,
            staged_nzb_dir,
        }
    }

    /// URL for the GraphQL endpoint.
    pub fn graphql_url(&self) -> String {
        format!("{}/graphql", self.app_url)
    }

    /// Build a reqwest client suitable for hitting the test server.
    pub fn http_client(&self) -> reqwest::Client {
        scryer_outbound_http::install_default_rustls_provider();
        reqwest::Client::builder()
            .build()
            .expect("failed to build reqwest client")
    }
}

async fn mount_default_smg_metadata_mocks(server: &MockServer) {
    let fixture = json!({
        "data": {
            "m0": {
                "movie": {
                    "tvdb_id": 123456,
                    "name": "Test Movie Title",
                    "slug": "test-movie-title",
                    "year": 2024,
                    "status": "Released",
                    "overview": "A gripping tale of testing integration.",
                    "poster_url": "https://artworks.thetvdb.com/banners/movies/123456/posters/test.jpg",
                    "language": "eng",
                    "runtime_minutes": 142,
                    "sort_title": "Test Movie Title",
                    "imdb_id": "tt1234567",
                    "genres": ["Action", "Thriller"],
                    "studio": "Test Studios",
                    "tmdb_release_date": "2024-06-15"
                }
            },
            "s0": {
                "series": {
                    "tvdb_id": 345678,
                    "name": "Test Show Name",
                    "sort_name": "Test Show Name",
                    "slug": "test-show-name",
                    "status": "Continuing",
                    "year": 2023,
                    "first_aired": "2023-09-15",
                    "overview": "A compelling drama about software testing.",
                    "network": "Test Network",
                    "runtime_minutes": 45,
                    "poster_url": "https://artworks.thetvdb.com/banners/series/345678/posters/test.jpg",
                    "country": "usa",
                    "genres": ["Drama", "Thriller"],
                    "aliases": ["Testing Show", "QA Chronicles"],
                    "tagged_aliases": [],
                    "seasons": [
                        {
                            "tvdb_id": 1000001,
                            "number": 1,
                            "label": "Season 1",
                            "episode_type": "default"
                        }
                    ],
                    "episodes": [
                        {
                            "tvdb_id": 2000001,
                            "episode_number": 1,
                            "season_number": 1,
                            "name": "Pilot",
                            "aired": "2023-09-15",
                            "runtime_minutes": 60,
                            "is_filler": false,
                            "is_recap": false,
                            "language": "eng",
                            "overview": "The team assembles.",
                            "absolute_number": "1"
                        }
                    ],
                    "anime_mappings": [],
                    "anime_movies": []
                }
            }
        }
    })
    .to_string();

    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .with_priority(100)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .with_priority(100)
        .mount(server)
        .await;
}

/// Load a fixture file relative to the workspace `tests/fixtures/` directory.
pub fn load_fixture(path: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let fixture_path = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("fixtures")
        .join(path);
    std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to load fixture {}: {e}", fixture_path.display()))
}

/// Build a minimal axum router with a GraphQL endpoint and authentication disabled.
fn build_test_router(
    app: AppUseCase,
    schema: ApiSchema,
    auth_runtime: AuthRuntimeStateHandle,
) -> Router {
    Router::new().route(
        "/graphql",
        post(test_graphql_handler).with_state((app, schema, auth_runtime)),
    )
}

/// Minimal GraphQL handler that replicates auth-disabled default-user injection.
async fn test_graphql_handler(
    State((app, schema, auth_runtime)): State<(AppUseCase, ApiSchema, AuthRuntimeStateHandle)>,
    req: GraphQLRequest,
) -> Response {
    let user = if auth_runtime.snapshot().effective_form_login_enabled {
        None
    } else {
        app.find_or_create_default_user().await.ok()
    };
    let mut request = req.into_inner();
    let response_status = graphql_response_status(&mut request);
    if let Some(u) = user {
        request = request.data(u);
    }
    let mut response =
        async_graphql_axum::GraphQLResponse::from(schema.execute(request).await).into_response();
    *response.status_mut() = response_status;
    response
}

fn graphql_response_status(request: &mut async_graphql::Request) -> StatusCode {
    let _ = request;
    StatusCode::OK
}
