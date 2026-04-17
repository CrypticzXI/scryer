#![allow(dead_code)]

use std::sync::Arc;

use async_graphql::parser::types::{DocumentOperations, OperationType, Selection};
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
use scryer_infrastructure::{
    FileSystemLibraryScanner, FileSystemStagedNzbStore, MetadataGatewayClient,
    MultiIndexerSearchClient, NzbgetDownloadClient, SmgEnrollmentConfig, SqliteCatalogStore,
    SqliteConfigStore, SqliteCustomizationStore, SqliteLibraryStateStore, SqliteReleaseStore,
    SqliteServices, SqliteSettingsStore,
};
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
    pub app: AppUseCase,
    pub catalog: SqliteCatalogStore,
    pub customization: SqliteCustomizationStore,
    pub library_state: SqliteLibraryStateStore,
    pub db: SqliteServices,
    pub settings_store: SqliteSettingsStore,
    pub staged_nzb_store: Arc<FileSystemStagedNzbStore>,
    pub staged_nzb_dir: tempfile::TempDir,
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
        let release_store = Arc::new(SqliteReleaseStore::new(&db));
        let settings_store = Arc::new(SqliteSettingsStore::new(&db));

        // Real clients pointed at wiremock URLs
        let nzbget = NzbgetDownloadClient::with_staged_nzb_store(
            nzbget_server.uri(),
            Some("test-user".to_string()),
            Some("test-pass".to_string()),
            "SCORE".to_string(),
            staged_nzb_store.clone(),
            staged_nzb_pipeline_limit.clone(),
        );

        let config_store = Arc::new(SqliteConfigStore::new(&db));

        // Build indexer client backed by built-in WASM plugins (using DynamicPluginProvider
        // so reload_plugins works in integration tests)
        let plugin_provider: Arc<dyn IndexerPluginProvider> =
            Arc::new(scryer_plugins::DynamicPluginProvider::new(
                scryer_plugins::WasmIndexerPluginProvider::empty()
                    .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM)
                    .with_builtin(scryer_plugins::builtins::NEWZNAB_WASM),
            ));
        let indexer_stats: Arc<dyn scryer_application::IndexerStatsTracker> = Arc::new(
            scryer_infrastructure::InMemoryIndexerStatsTracker::new(None),
        );
        let indexer_client = MultiIndexerSearchClient::new(
            config_store.clone(),
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
        let catalog_store = SqliteCatalogStore::new(&db);
        let titles: Arc<dyn scryer_application::TitleRepository> = Arc::new(catalog_store.clone());
        let shows: Arc<dyn scryer_application::ShowRepository> = Arc::new(catalog_store.clone());
        let users: Arc<dyn scryer_application::UserRepository> = Arc::new(catalog_store.clone());
        let indexer_configs: Arc<dyn scryer_application::IndexerConfigRepository> =
            config_store.clone();
        let download_client_configs: Arc<dyn scryer_application::DownloadClientConfigRepository> =
            config_store.clone();
        let release_attempts: Arc<dyn scryer_application::ReleaseAttemptRepository> = release_store;
        let settings: Arc<dyn scryer_application::SettingsRepository> = settings_store.clone();
        let quality_profiles: Arc<dyn scryer_application::QualityProfileRepository> =
            settings_store.clone();

        let library_state_store = SqliteLibraryStateStore::new(&db);
        let customization_store = SqliteCustomizationStore::new(&db);
        let workflow_store = Arc::new(scryer_infrastructure::SqliteWorkflowStore::new(&db));
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
        .with_library_state_store(Arc::new(library_state_store.clone()))
        .with_customization_store(Arc::new(customization_store.clone()))
        .with_acquisition_state(workflow_store.clone())
        .with_domain_events(workflow_store.clone())
        .with_download_submissions(workflow_store.clone())
        .with_import_artifacts(workflow_store.clone())
        .with_imports(workflow_store.clone())
        .with_job_runs(workflow_store.clone())
        .with_system_info(settings_store.clone())
        .with_metadata_gateway(Arc::new(metadata_gateway))
        .with_library_scanner(Arc::new(FileSystemLibraryScanner::new()))
        .with_indexer_stats(indexer_stats)
        .with_plugin_provider(plugin_provider)
        .with_staged_nzb_store(staged_nzb_store.clone())
        .with_staged_nzb_pipeline_limit(staged_nzb_pipeline_limit)
        .with_workflow_operations(workflow_store)
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
        let schema = build_schema(app.clone(), false);

        // Start axum server on a random port
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind test server");
        let addr = listener.local_addr().expect("failed to get local addr");
        let app_url = format!("http://{addr}");

        let router = build_test_router(app.clone(), schema.clone());
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
            app,
            catalog: catalog_store,
            customization: customization_store,
            library_state: library_state_store,
            db,
            settings_store: settings_store.as_ref().clone(),
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
fn build_test_router(app: AppUseCase, schema: ApiSchema) -> Router {
    Router::new().route(
        "/graphql",
        post(test_graphql_handler).with_state((app, schema)),
    )
}

/// Minimal GraphQL handler that replicates auth-disabled default-user injection.
async fn test_graphql_handler(
    State((app, schema)): State<(AppUseCase, ApiSchema)>,
    req: GraphQLRequest,
) -> Response {
    let user = app.find_or_create_default_user().await.ok();
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
    graphql_async_mutation_status(request).unwrap_or(StatusCode::OK)
}

fn graphql_async_mutation_status(request: &mut async_graphql::Request) -> Option<StatusCode> {
    let operation_name = request.operation_name.clone();
    let Ok(document) = request.parsed_query() else {
        return None;
    };

    let operation = match (&document.operations, operation_name.as_deref()) {
        (DocumentOperations::Single(operation), _) => operation,
        (DocumentOperations::Multiple(operations), Some(operation_name)) => {
            operations.get(operation_name)?
        }
        (DocumentOperations::Multiple(operations), None) => {
            if operations.len() != 1 {
                return None;
            }

            operations.values().next()?
        }
    };

    if operation.node.ty != OperationType::Mutation {
        return None;
    }

    let field_names = operation
        .node
        .selection_set
        .node
        .items
        .iter()
        .filter_map(|selection| match &selection.node {
            Selection::Field(field) => Some(field.node.name.node.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();

    if field_names.contains(&"rehydrateAllMetadata") {
        return Some(StatusCode::ACCEPTED);
    }

    if field_names.contains(&"queueManualImport") {
        return Some(StatusCode::ACCEPTED);
    }

    if field_names.contains(&"scanLibrary") {
        return Some(StatusCode::CREATED);
    }

    None
}
