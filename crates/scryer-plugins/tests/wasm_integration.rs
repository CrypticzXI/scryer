use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use chrono::Utc;
use scryer_application::IndexerPluginProvider;
use scryer_domain::IndexerConfig;

fn test_config(provider_type: &str) -> IndexerConfig {
    IndexerConfig {
        id: "idx-1".to_string(),
        name: "Test".to_string(),
        provider_type: provider_type.to_string(),
        base_url: "https://example.com".to_string(),
        api_key_encrypted: None,
        rate_limit_seconds: None,
        rate_limit_burst: None,
        disabled_until: None,
        is_enabled: true,
        enable_interactive_search: true,
        enable_auto_search: true,
        last_health_status: None,
        last_error_at: None,
        config_json: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn load_test_indexer_plugin() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    let types = provider.available_provider_types();
    assert_eq!(types, vec!["test"]);
}

#[test]
fn test_indexer_creates_client() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    let client = provider.client_for_provider(&test_config("test"));
    assert!(
        client.is_some(),
        "should create a client for provider_type 'test'"
    );
}

#[test]
fn unknown_provider_returns_none() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    assert!(
        provider
            .client_for_provider(&test_config("nonexistent"))
            .is_none()
    );
}

#[tokio::test]
async fn test_indexer_search() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    let client = provider.client_for_provider(&test_config("test")).unwrap();

    use scryer_application::SearchMode;
    let results = client
        .search(
            "Dune Part Two".to_string(),
            std::collections::HashMap::new(),
            None,
            None,
            None,
            None,
            SearchMode::Auto,
            None,
            None,
            None,
            vec![],
        )
        .await
        .unwrap()
        .results;

    assert_eq!(results.len(), 1);
    let r = &results[0];
    assert!(r.title.contains("Dune Part Two"));
    assert_eq!(r.size_bytes, Some(8_000_000_000));
    assert!(r.source.contains("Test"));
}

#[test]
fn empty_dir_loads_no_plugins() {
    let tmp = tempfile::tempdir().unwrap();
    let provider = scryer_plugins::load_indexer_plugins(tmp.path()).unwrap();
    assert!(provider.available_provider_types().is_empty());
}

#[test]
fn scoring_policies_empty_for_test_plugin() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();
    // The test-indexer fixture has no scoring policies
    assert!(provider.scoring_policies().is_empty());
}

// ── WasmIndexerPluginProvider builder tests ──────────────────────────────────

#[test]
fn builtin_provider_exposes_expected_metadata_and_supports_removal() {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM)
        .with_builtin(scryer_plugins::builtins::NEWZNAB_WASM);

    let mut types = provider.available_provider_types();
    types.sort();
    assert!(
        types.contains(&"nzbgeek".to_string()),
        "nzbgeek should register"
    );
    assert!(
        types.contains(&"newznab".to_string()),
        "newznab should register"
    );

    assert!(
        provider.plugin_name_for_provider("nzbgeek").is_some(),
        "nzbgeek should have a plugin name"
    );
    assert!(
        provider.plugin_name_for_provider("newznab").is_some(),
        "newznab should have a plugin name"
    );
    assert_eq!(
        provider.default_base_url_for_provider("nzbgeek").as_deref(),
        Some("https://api.nzbgeek.info"),
        "nzbgeek should expose its default base URL"
    );
    assert!(
        provider.default_base_url_for_provider("newznab").is_none(),
        "newznab should not expose a default base URL"
    );

    let trimmed = provider.without_provider_type("nzbgeek");
    let trimmed_types = trimmed.available_provider_types();
    assert!(
        !trimmed_types.contains(&"nzbgeek".to_string()),
        "without_provider_type should drop nzbgeek"
    );
    assert!(
        trimmed_types.contains(&"newznab".to_string()),
        "without_provider_type should leave newznab intact"
    );
}

#[test]
fn external_overrides_builtin_same_provider() {
    // Load test fixture (provider_type = "test") as external, then try
    // loading it again as builtin — builtin should be skipped.
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let wasm_bytes = std::fs::read(fixtures_dir.join("test-indexer/plugin.wasm")).unwrap();

    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_external_bytes(&wasm_bytes)
        .with_builtin(&wasm_bytes); // same provider_type — should be skipped

    // Only one entry for "test", not duplicated
    let types = provider.available_provider_types();
    assert_eq!(
        types.iter().filter(|t| *t == "test").count(),
        1,
        "builtin should not duplicate external"
    );
}

#[test]
fn invalid_wasm_bytes_silently_skipped() {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_external_bytes(b"this is not valid wasm");

    assert!(
        provider.available_provider_types().is_empty(),
        "invalid WASM should be skipped"
    );
}

#[test]
fn invalid_bytes_dont_affect_valid() {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM)
        .with_external_bytes(b"garbage");

    let types = provider.available_provider_types();
    assert!(
        types.contains(&"nzbgeek".to_string()),
        "valid builtin should survive despite garbage external"
    );
}

#[test]
fn dognzb_hides_generic_newznab_config_fields() {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::DOGNZB_WASM);

    assert_eq!(
        provider.default_base_url_for_provider("dognzb").as_deref(),
        Some("https://api.dognzb.cr")
    );

    let fields = provider.config_fields_for_provider("dognzb");
    let field_keys: Vec<&str> = fields.iter().map(|field| field.key.as_str()).collect();

    assert!(
        !field_keys.contains(&"api_path"),
        "DogNZB should not expose api_path"
    );
    assert!(
        !field_keys.contains(&"additional_params"),
        "DogNZB should not expose additional_params"
    );
}

#[test]
fn newznab_family_builtins_include_rss_search_path() {
    for (name, wasm_bytes) in [
        ("nzbgeek", scryer_plugins::builtins::NZBGEEK_WASM),
        ("dognzb", scryer_plugins::builtins::DOGNZB_WASM),
        ("newznab", scryer_plugins::builtins::NEWZNAB_WASM),
        ("torznab", scryer_plugins::builtins::TORZNAB_WASM),
    ] {
        assert!(
            bytes_contain(wasm_bytes, b"rss_search: fetching recent releases"),
            "{name} builtin WASM is missing the RSS search path"
        );
    }
}

#[tokio::test]
async fn nzbgeek_builtin_rss_search_uses_category_only_request() {
    let (base_url, request_rx) = spawn_newznab_response_server();
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM);

    let mut config = test_config("nzbgeek");
    config.base_url = base_url;
    config.api_key_encrypted = Some("test-key".to_string());

    let client = provider.client_for_provider(&config).unwrap();
    let response = client
        .search(
            String::new(),
            std::collections::HashMap::new(),
            None,
            Some("series".to_string()),
            Some(vec!["5000".to_string()]),
            None,
            scryer_application::SearchMode::Auto,
            None,
            None,
            None,
            vec![],
        )
        .await
        .unwrap();

    assert_eq!(response.results.len(), 1);
    assert_eq!(
        response.results[0].title,
        "Example.Show.S01E01.1080p.WEB-DL"
    );

    let request = request_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("mock Newznab server should receive a request");
    assert!(request.contains("GET /api?"), "request was {request}");
    assert!(request.contains("t=tvsearch"), "request was {request}");
    assert!(request.contains("cat=5000"), "request was {request}");
    assert!(
        !request.contains("q="),
        "RSS request should not include q=: {request}"
    );
}

fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn spawn_newznab_response_server() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = mpsc::channel();

    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buffer = [0_u8; 8192];
                    let bytes_read = stream.read(&mut buffer).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
                    let _ = request_tx.send(request);

                    let body = r#"{"channel":{"item":[{"title":"Example.Show.S01E01.1080p.WEB-DL","guid":"guid-1","link":"http://example.test/info","enclosure":{"@attributes":{"url":"http://example.test/download.nzb","length":"12345","type":"application/x-nzb"}},"attr":[{"@attributes":{"name":"grabs","value":"4"}}]}]}}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });

    (format!("http://{address}"), request_rx)
}

// ── DynamicPluginProvider tests ──────────────────────────────────────────────

#[test]
fn dynamic_delegates_available_types() {
    let inner = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM);
    let dynamic = scryer_plugins::DynamicPluginProvider::new(inner);

    let types = dynamic.available_provider_types();
    assert!(types.contains(&"nzbgeek".to_string()));
}

#[test]
fn dynamic_provider_reload_behaviour() {
    let inner = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM)
        .with_builtin(scryer_plugins::builtins::NEWZNAB_WASM);
    let dynamic = scryer_plugins::DynamicPluginProvider::new(inner);

    assert_eq!(
        dynamic.available_provider_types().len(),
        2,
        "dynamic should initially expose both builtins"
    );

    // reload_plugins disables a single provider while keeping the rest.
    dynamic
        .reload_plugins(&[], &["nzbgeek".to_string()])
        .unwrap();
    let after_disable = dynamic.available_provider_types();
    assert!(
        !after_disable.contains(&"nzbgeek".to_string()),
        "nzbgeek should be disabled after reload_plugins"
    );
    assert!(
        after_disable.contains(&"newznab".to_string()),
        "newznab should remain after reload_plugins"
    );

    // reload swaps the inner provider entirely; an empty provider clears all.
    dynamic.reload(scryer_plugins::WasmIndexerPluginProvider::empty());
    assert!(
        dynamic.available_provider_types().is_empty(),
        "after reload with empty provider, no types should remain"
    );
}

#[test]
fn dynamic_client_cache_hit() {
    let inner = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM);
    let dynamic = scryer_plugins::DynamicPluginProvider::new(inner);

    let config = test_config("nzbgeek");
    let c1 = dynamic.client_for_provider(&config).unwrap();
    let c2 = dynamic.client_for_provider(&config).unwrap();
    assert!(
        Arc::ptr_eq(&c1, &c2),
        "same config should return cached client"
    );
}

#[test]
fn dynamic_client_cache_miss_on_updated_at() {
    let inner = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM);
    let dynamic = scryer_plugins::DynamicPluginProvider::new(inner);

    let mut config1 = test_config("nzbgeek");
    let c1 = dynamic.client_for_provider(&config1).unwrap();

    // Change updated_at to simulate a config update
    config1.updated_at = Utc::now() + chrono::Duration::seconds(10);
    let c2 = dynamic.client_for_provider(&config1).unwrap();

    assert!(
        !Arc::ptr_eq(&c1, &c2),
        "different updated_at should produce a new client"
    );
}

// ── Builder validation tests ─────────────────────────────────────────────────

#[test]
fn builtin_with_valid_descriptor_loads() {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM);

    assert!(
        provider
            .available_provider_types()
            .contains(&"nzbgeek".to_string()),
        "NZBGEEK_WASM should register as 'nzbgeek'"
    );
}

#[test]
fn plugin_capabilities_accessible() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    let caps = provider.capabilities_for_provider("test");
    assert!(caps.rss, "rss capability should default to true");
    // The test plugin should have some capabilities declared
    // (the default is all-true, so at minimum search should be true)
    assert!(caps.search, "search capability should be true");
}
