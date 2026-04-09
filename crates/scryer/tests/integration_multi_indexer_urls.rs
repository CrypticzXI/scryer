//! End-to-end integration test that runs the full search pipeline
//! (discovery → multi-indexer → WASM plugins → HTTP) and captures
//! every outbound URL from real plugin binaries.
//!
//! Run with: cargo nextest run -E 'test(multi_indexer_url_trace)' --success-output immediate

mod common;

use std::sync::Arc;

use common::load_fixture;
use scryer_application::{
    AppServices, AppUseCase, FacetRegistry, IndexerPluginProvider, JwtAuthConfig,
    MovieFacetHandler, SeriesFacetHandler,
};
use scryer_domain::{Entitlement, User};
use scryer_infrastructure::{
    FileSystemLibraryScanner, InMemoryIndexerStatsTracker, MultiIndexerSearchClient,
    SqliteCatalogStore, SqliteConfigStore, SqliteCustomizationStore, SqliteLibraryStateStore,
    SqliteReleaseStore, SqliteServices, SqliteSettingsStore, SqliteWorkflowStore,
};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOSHO_EMPTY: &str = "[]";
const NEWZNAB_EMPTY: &str = r#"{"channel":{"item":[]}}"#;

/// Build a full AppUseCase with AnimeTosho, NZBGeek, and Torznab plugins,
/// each backed by its own wiremock server. Creates indexer configs in SQLite
/// so the multi-indexer discovers them at search time.
async fn setup() -> (
    AppUseCase,
    User,
    MockServer, // tosho
    MockServer, // nzbgeek
    MockServer, // torznab
) {
    let tosho_server = MockServer::start().await;
    let nzbgeek_server = MockServer::start().await;
    let torznab_server = MockServer::start().await;

    // Mount catch-all empty responses
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(TOSHO_EMPTY))
        .mount(&tosho_server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(NEWZNAB_EMPTY))
        .mount(&nzbgeek_server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(NEWZNAB_EMPTY))
        .mount(&torznab_server)
        .await;

    let db = SqliteServices::new(":memory:")
        .await
        .expect("in-memory SQLite");
    let config_store = Arc::new(SqliteConfigStore::new(&db));
    let release_store = Arc::new(SqliteReleaseStore::new(&db));
    let settings_store = Arc::new(SqliteSettingsStore::new(&db));
    let workflow_store = Arc::new(SqliteWorkflowStore::new(&db));

    // Load all three indexer plugins
    let plugin_provider: Arc<dyn IndexerPluginProvider> =
        Arc::new(scryer_plugins::DynamicPluginProvider::new(
            scryer_plugins::WasmIndexerPluginProvider::empty()
                .with_builtin(scryer_plugins::builtins::ANIMETOSHO_WASM)
                .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM)
                .with_builtin(scryer_plugins::builtins::TORZNAB_WASM),
        ));

    let indexer_stats: Arc<dyn scryer_application::IndexerStatsTracker> =
        Arc::new(InMemoryIndexerStatsTracker::new(None));

    let indexer_client = MultiIndexerSearchClient::new(
        config_store.clone(),
        indexer_stats.clone(),
        plugin_provider.clone(),
    );

    // Create indexer configs in SQLite so the multi-indexer finds them
    use scryer_application::IndexerConfigRepository;
    let now = chrono::Utc::now();
    for config in [
        scryer_domain::IndexerConfig {
            id: "tosho-1".into(),
            name: "AnimeTosho".into(),
            provider_type: "animetosho".into(),
            base_url: tosho_server.uri(),
            api_key_encrypted: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            rate_limit_seconds: Some(0),
            rate_limit_burst: None,
            disabled_until: None,
            last_health_status: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        },
        scryer_domain::IndexerConfig {
            id: "nzbgeek-1".into(),
            name: "NZBGeek".into(),
            provider_type: "nzbgeek".into(),
            base_url: format!("{}/api", nzbgeek_server.uri()),
            api_key_encrypted: Some("test-api-key".into()),
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            rate_limit_seconds: Some(0),
            rate_limit_burst: None,
            disabled_until: None,
            last_health_status: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        },
        scryer_domain::IndexerConfig {
            id: "torznab-1".into(),
            name: "Torznab".into(),
            provider_type: "torznab".into(),
            base_url: format!("{}/api", torznab_server.uri()),
            api_key_encrypted: Some("test-api-key".into()),
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            rate_limit_seconds: Some(0),
            rate_limit_burst: None,
            disabled_until: None,
            last_health_status: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        },
    ] {
        config_store
            .create(config)
            .await
            .expect("create indexer config");
    }

    scryer_application::DownloadClientConfigRepository::create(
        &*config_store,
        scryer_domain::DownloadClientConfig {
            id: "nzbget-1".into(),
            name: "NZBGet".into(),
            client_type: "nzbget".into(),
            config_json: serde_json::json!({
                "base_url": "http://localhost:1"
            })
            .to_string(),
            client_priority: 1,
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::default(),
            last_error: None,
            last_seen_at: None,
            created_at: now,
            updated_at: now,
        },
    )
    .await
    .expect("create download client config");

    // Build a minimal download client so AppServices doesn't panic
    let staged_nzb_dir = tempfile::TempDir::new().expect("failed to create staged nzb tempdir");
    let staged_nzb_store = Arc::new(
        scryer_infrastructure::FileSystemStagedNzbStore::new(staged_nzb_dir.path())
            .await
            .expect("staged nzb store"),
    );
    let nzbget = scryer_infrastructure::NzbgetDownloadClient::with_staged_nzb_store(
        "http://localhost:1".to_string(),
        None,
        None,
        "SCORE".to_string(),
        staged_nzb_store.clone(),
        Arc::new(tokio::sync::Semaphore::new(4)),
    );

    let smg = scryer_infrastructure::MetadataGatewayClient::new(
        "http://localhost:2/graphql".to_string(),
        true,
        db.clone(),
        scryer_infrastructure::SmgEnrollmentConfig {
            registration_secret: None,
            ca_cert: None,
        },
    );

    let catalog_store = Arc::new(SqliteCatalogStore::new(&db));
    let titles: Arc<dyn scryer_application::TitleRepository> = catalog_store.clone();
    let shows: Arc<dyn scryer_application::ShowRepository> = catalog_store.clone();
    let users: Arc<dyn scryer_application::UserRepository> = catalog_store;
    let indexer_configs_repo: Arc<dyn scryer_application::IndexerConfigRepository> =
        config_store.clone();
    let download_client_configs: Arc<dyn scryer_application::DownloadClientConfigRepository> =
        config_store.clone();
    let release_attempts: Arc<dyn scryer_application::ReleaseAttemptRepository> = release_store;
    let settings: Arc<dyn scryer_application::SettingsRepository> = settings_store.clone();
    let quality_profiles: Arc<dyn scryer_application::QualityProfileRepository> =
        settings_store.clone();

    let library_state_store = Arc::new(SqliteLibraryStateStore::new(&db));
    let customization_store = Arc::new(SqliteCustomizationStore::new(&db));
    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs_repo,
        Arc::new(indexer_client),
        Arc::new(nzbget),
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        ":memory:".to_string(),
    )
    .with_library_state_store(library_state_store)
    .with_customization_store(customization_store)
    .with_acquisition_state(workflow_store.clone())
    .with_domain_events(workflow_store.clone())
    .with_download_submissions(workflow_store.clone())
    .with_import_artifacts(workflow_store.clone())
    .with_imports(workflow_store.clone())
    .with_job_runs(workflow_store.clone())
    .with_system_info(settings_store)
    .with_metadata_gateway(Arc::new(smg))
    .with_library_scanner(Arc::new(FileSystemLibraryScanner::new()))
    .with_indexer_stats(indexer_stats)
    .with_plugin_provider(plugin_provider)
    .with_staged_nzb_store(staged_nzb_store)
    .with_workflow_operations(workflow_store)
    .build();

    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));

    let app = AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "scryer-test".into(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".into(),
        },
        Arc::new(registry),
    );

    // Create a test user with ViewCatalog entitlement
    let user = User {
        id: "test-user".into(),
        username: "tester".into(),
        password_hash: None,
        entitlements: vec![Entitlement::ViewCatalog],
    };

    (app, user, tosho_server, nzbgeek_server, torznab_server)
}

async fn captured_urls(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| r.url.to_string())
        .collect()
}

fn print_urls(label: &str, urls: &[String]) {
    if urls.is_empty() {
        println!("  {label}: (no calls)");
    } else {
        for url in urls {
            println!("  {label}: {url}");
        }
    }
}

fn print_summary(tosho: &[String], nzbgeek: &[String], torznab: &[String]) {
    print_urls("AnimeTosho", tosho);
    print_urls("NZBGeek", nzbgeek);
    print_urls("Torznab", torznab);
    println!(
        "  Total HTTP calls: {}",
        tosho.len() + nzbgeek.len() + torznab.len()
    );
}

fn assert_id_only_then_fallback(urls: &[String], id_fragment: &str, fallback_query_fragment: &str) {
    assert!(
        !urls.is_empty(),
        "expected at least one request containing {id_fragment}"
    );
    assert!(
        urls[0].contains(id_fragment),
        "first request should use ID search: {:?}",
        urls
    );
    assert!(
        !urls[0].contains("&q="),
        "first request should not mix freetext into the ID tier: {:?}",
        urls[0]
    );
    assert!(
        urls.iter()
            .skip(1)
            .any(|url| url.contains(fallback_query_fragment)),
        "expected a later freetext fallback request containing {fallback_query_fragment}: {:?}",
        urls
    );
}

// ---------------------------------------------------------------------------
// Demon Slayer S02E03 — anime episode, end-to-end through discovery layer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_indexer_url_trace_anime_episode() {
    let (app, user, tosho, nzbgeek, torznab) = setup().await;

    let _results = app
        .search_indexers_episode(
            &user,
            scryer_application::IndexerEpisodeSearchRequest {
                title: "Demon Slayer".into(),
                season: "02".into(),
                episode: "03".into(),
                imdb_id: None,
                tvdb_id: Some("348545".into()),
                anidb_id: Some("1535".into()),
                category: Some("anime".into()),
                absolute_episode: None,
            },
        )
        .await
        .expect("search should succeed");

    println!("\n=== Demon Slayer S02E03 (anime, anidb=1535, tvdb=348545) ===");
    print_summary(
        &captured_urls(&tosho).await,
        &captured_urls(&nzbgeek).await,
        &captured_urls(&torznab).await,
    );
}

// ---------------------------------------------------------------------------
// Breaking Bad S05E01 — regular TV series
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_indexer_url_trace_tv_episode() {
    let (app, user, tosho, nzbgeek, torznab) = setup().await;

    let _results = app
        .search_indexers_episode(
            &user,
            scryer_application::IndexerEpisodeSearchRequest {
                title: "Breaking Bad".into(),
                season: "05".into(),
                episode: "01".into(),
                imdb_id: None,
                tvdb_id: Some("81189".into()),
                anidb_id: None,
                category: Some("series".into()),
                absolute_episode: None,
            },
        )
        .await
        .expect("search should succeed");

    let tosho_urls = captured_urls(&tosho).await;
    let nzbgeek_urls = captured_urls(&nzbgeek).await;
    let torznab_urls = captured_urls(&torznab).await;

    println!("\n=== Breaking Bad S05E01 (series, tvdb=81189) ===");
    print_summary(&tosho_urls, &nzbgeek_urls, &torznab_urls);

    assert!(
        tosho_urls.is_empty(),
        "AnimeTosho should not handle series searches"
    );
    assert_id_only_then_fallback(&nzbgeek_urls, "tvdbid=81189", "q=Breaking%20Bad");
    assert_id_only_then_fallback(&torznab_urls, "tvdbid=81189", "q=Breaking%20Bad");
}

// ---------------------------------------------------------------------------
// The Matrix — movie with imdb_id only
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_indexer_url_trace_movie() {
    let (app, user, tosho, nzbgeek, torznab) = setup().await;

    let _results = app
        .search_indexers(
            &user,
            scryer_application::IndexerSearchRequest {
                query: "The Matrix".into(),
                imdb_id: Some("tt0133093".into()),
                tvdb_id: None,
                anidb_id: None,
                category: Some("movie".into()),
            },
        )
        .await
        .expect("search should succeed");

    let tosho_urls = captured_urls(&tosho).await;
    let nzbgeek_urls = captured_urls(&nzbgeek).await;
    let torznab_urls = captured_urls(&torznab).await;

    println!("\n=== The Matrix (movie, imdb=tt0133093) ===");
    print_summary(&tosho_urls, &nzbgeek_urls, &torznab_urls);

    assert!(
        tosho_urls.is_empty(),
        "AnimeTosho should not handle non-anime movie searches"
    );
    assert_id_only_then_fallback(&nzbgeek_urls, "imdbid=000133093", "q=The%20Matrix");
    assert_id_only_then_fallback(&torznab_urls, "imdbid=000133093", "q=The%20Matrix");
}

// ---------------------------------------------------------------------------
// Spirited Away — movie with imdb_id + anidb_id (from metadata hydration)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_indexer_url_trace_movie_spirited_away() {
    let (app, user, tosho, nzbgeek, torznab) = setup().await;

    let nzbgeek_fixture = load_fixture("nzbgeek/search_movie.json").replace(
        "Movie.Title.2024.2160p.UHD.BluRay.REMUX.DV.HDR.DTS-HD.MA.7.1.HEVC-GROUP",
        "Sen.to.Chihiro.no.Kamikakushi.2001.1080p.BluRay",
    );
    Mock::given(method("GET"))
        .and(path("/api/api"))
        .and(query_param("imdbid", "000245429"))
        .respond_with(ResponseTemplate::new(200).set_body_string(nzbgeek_fixture))
        .with_priority(1)
        .mount(&nzbgeek)
        .await;

    let results = app
        .search_indexers(
            &user,
            scryer_application::IndexerSearchRequest {
                query: "Spirited Away".into(),
                imdb_id: Some("tt0245429".into()),
                tvdb_id: None,
                anidb_id: Some("112".into()),
                category: Some("movie".into()),
            },
        )
        .await
        .expect("search should succeed");

    let tosho_urls = captured_urls(&tosho).await;
    let nzbgeek_urls = captured_urls(&nzbgeek).await;
    let torznab_urls = captured_urls(&torznab).await;

    assert!(
        results
            .iter()
            .any(|result| result.title.contains("Sen.to.Chihiro.no.Kamikakushi")),
        "ID-backed alternate title should survive the title guard, got {:?}, urls: tosho={:?}, nzbgeek={:?}, torznab={:?}",
        results
            .iter()
            .map(|result| result.title.clone())
            .collect::<Vec<_>>(),
        tosho_urls,
        nzbgeek_urls,
        torznab_urls
    );

    println!("\n=== Spirited Away (movie, imdb=tt0245429, anidb=112) ===");
    print_summary(&tosho_urls, &nzbgeek_urls, &torznab_urls);
}

// ---------------------------------------------------------------------------
// Demon Slayer Season 2 pack — background acquisition path
// (season=Some, episode=None, query=title name only)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_indexer_url_trace_season_pack() {
    let (app, user, tosho, nzbgeek, torznab) = setup().await;

    // Call the indexer client directly as the background acquisition loop would.
    // season=Some(2), episode=None signals a season pack search.
    // The acquisition loop builds "Title S02" as the query.
    let _results = app
        .search_indexers_season(
            &user,
            scryer_application::IndexerSeasonSearchRequest {
                title: "Demon Slayer".into(),
                season: "02".into(),
                imdb_id: None,
                tvdb_id: Some("348545".into()),
                anidb_id: Some("1535".into()),
                category: Some("anime".into()),
            },
        )
        .await
        .expect("search should succeed");

    println!(
        "\n=== Demon Slayer Season 2 Pack (anime, tvdb=348545, anidb=1535, season=2, ep=None) ==="
    );
    print_summary(
        &captured_urls(&tosho).await,
        &captured_urls(&nzbgeek).await,
        &captured_urls(&torznab).await,
    );
}
