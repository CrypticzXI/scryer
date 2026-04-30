use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use extism::Manifest;
use scryer_application::{
    DownloadClient, DownloadClientPluginProvider, ExternalPluginWasm, IndexerClient,
    IndexerPluginProvider, NotificationClient, NotificationPluginProvider, SubtitlePluginProvider,
    SubtitleProviderClient,
};
use scryer_domain::{
    DownloadClientConfig, IndexerConfig, NotificationChannelConfig, PluginHostBindingId,
    SubtitleProviderConfig,
};
use tracing::{info, warn};

use crate::download_client_adapter::WasmDownloadClient;
use crate::indexer_adapter::WasmIndexerClient;
use crate::notification_adapter::WasmNotificationClient;
use crate::subtitle_adapter::WasmSubtitleClient;
use crate::types::{
    ConfigFieldValueSource, EXPORT_DESCRIBE, EXPORT_DOWNLOAD_ADD, EXPORT_DOWNLOAD_CONTROL,
    EXPORT_DOWNLOAD_LIST_COMPLETED, EXPORT_DOWNLOAD_LIST_HISTORY, EXPORT_DOWNLOAD_LIST_QUEUE,
    EXPORT_DOWNLOAD_MARK_IMPORTED, EXPORT_DOWNLOAD_STATUS, EXPORT_DOWNLOAD_TEST_CONNECTION,
    EXPORT_INDEXER_SEARCH, EXPORT_NOTIFICATION_SEND, EXPORT_SUBTITLE_DOWNLOAD,
    EXPORT_SUBTITLE_GENERATE, EXPORT_SUBTITLE_SEARCH, EXPORT_VALIDATE_CONFIG, PluginDescriptor,
    PluginHostBindingId as SdkHostBinding, PluginKind, ProviderDescriptor, SDK_VERSION,
    SubtitleProviderMode, config_fields_to_domain, indexer_capabilities_to_domain,
    plugin_descriptor_sdk_constraint, validate_plugin_descriptor_sdk_contract,
};

const NZBGEEK_DEFAULT_BASE_URL: &str = "https://api.nzbgeek.info";
const DOGNZB_DEFAULT_BASE_URL: &str = "https://api.dognzb.cr";
const INDEXER_PLUGIN_TYPES: &[&str] = &["indexer", "usenet_indexer", "torrent_indexer"];

type SubtitleClientCacheKey = (String, String, String, String);
type SubtitleClientCache =
    std::sync::Mutex<HashMap<SubtitleClientCacheKey, Arc<dyn SubtitleProviderClient>>>;

static WASMTIME_PLUGIN_BUILD_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PluginLoadSource {
    Builtin,
    External { first_party: bool },
}

impl PluginLoadSource {
    fn can_use_first_party_host_bindings(self) -> bool {
        matches!(self, Self::Builtin | Self::External { first_party: true })
    }
}

struct LoadedPlugin {
    wasm_bytes: Vec<u8>,
    descriptor: PluginDescriptor,
}

fn builtin_provider_types_from_bytes(
    wasm_bytes: &[&[u8]],
    plugin_type_filter: impl Fn(&str) -> bool,
    apply_overrides: impl Fn(PluginDescriptor) -> PluginDescriptor,
) -> Vec<String> {
    wasm_bytes
        .iter()
        .filter_map(|bytes| {
            load_from_bytes(bytes)
                .ok()
                .map(|(descriptor, _)| descriptor)
        })
        .map(apply_overrides)
        .filter(|descriptor| plugin_type_filter(descriptor.plugin_type()))
        .map(|descriptor| descriptor.provider_type().trim().to_ascii_lowercase())
        .collect()
}

fn builtin_indexer_provider_types() -> Vec<String> {
    static BUILTIN_INDEXER_PROVIDER_TYPES: LazyLock<Vec<String>> = LazyLock::new(|| {
        builtin_provider_types_from_bytes(
            &[
                crate::builtins::NZBGEEK_WASM,
                crate::builtins::NEWZNAB_WASM,
                crate::builtins::ANIMETOSHO_WASM,
                crate::builtins::TORZNAB_WASM,
            ],
            is_indexer_plugin_type,
            apply_builtin_indexer_overrides,
        )
    });

    BUILTIN_INDEXER_PROVIDER_TYPES.clone()
}

fn builtin_subtitle_provider_types() -> Vec<String> {
    static BUILTIN_SUBTITLE_PROVIDER_TYPES: LazyLock<Vec<String>> = LazyLock::new(|| {
        builtin_provider_types_from_bytes(
            &[crate::builtins::JIMAKU_WASM],
            |plugin_type| plugin_type == "subtitle_provider",
            |descriptor| descriptor,
        )
    });

    BUILTIN_SUBTITLE_PROVIDER_TYPES.clone()
}

pub struct WasmIndexerPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
}

impl WasmIndexerPluginProvider {
    /// Create an empty provider with no plugins loaded.
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    /// Register an externally-installed plugin from WASM bytes.
    /// External plugins take priority over built-ins with the same provider_type.
    pub fn with_external_bytes(self, wasm_bytes: &[u8]) -> Self {
        self.with_external_plugin(ExternalPluginWasm {
            bytes: wasm_bytes,
            first_party: false,
        })
    }

    fn with_external_plugin(mut self, plugin: ExternalPluginWasm<'_>) -> Self {
        match load_from_bytes(plugin.bytes) {
            Ok((descriptor, bytes)) => {
                if !validate_indexer_descriptor(
                    &descriptor,
                    PluginLoadSource::External {
                        first_party: plugin.first_party,
                    },
                ) {
                    return self;
                }

                let provider_type = descriptor.provider_type().trim().to_ascii_lowercase();
                let aliases: Vec<String> = descriptor
                    .provider_aliases()
                    .iter()
                    .map(|a| a.trim().to_ascii_lowercase())
                    .collect();

                info!(
                    plugin = descriptor.name.as_str(),
                    version = descriptor.version.as_str(),
                    provider_type = provider_type.as_str(),
                    "registered external plugin"
                );
                self.plugins.insert(
                    provider_type.clone(),
                    LoadedPlugin {
                        wasm_bytes: bytes.clone(),
                        descriptor: descriptor.clone(),
                    },
                );

                for alias in &aliases {
                    self.plugins.insert(
                        alias.clone(),
                        LoadedPlugin {
                            wasm_bytes: bytes.clone(),
                            descriptor: descriptor.clone(),
                        },
                    );
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to load external plugin");
            }
        }
        self
    }

    /// Remove a provider_type (and its aliases) from the loaded set.
    /// Used to disable built-in plugins at runtime.
    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let key = provider_type.trim().to_ascii_lowercase();
        if let Some(loaded) = self.plugins.remove(&key) {
            info!(
                plugin = loaded.descriptor.name.as_str(),
                provider_type = key.as_str(),
                "removed plugin provider_type"
            );
            // Also remove any aliases that point to the same descriptor
            let aliases: Vec<String> = loaded
                .descriptor
                .provider_aliases()
                .iter()
                .map(|a| a.trim().to_ascii_lowercase())
                .collect();
            for alias in &aliases {
                self.plugins.remove(alias);
            }
        }
        self
    }

    /// Register a built-in plugin from WASM bytes. The plugin is loaded,
    /// validated, and registered under its `provider_type` (and any
    /// `provider_aliases`). If an external plugin already claims the same
    /// provider_type, the external one wins and the built-in is skipped
    /// for that key.
    pub fn with_builtin(self, wasm_bytes: &[u8]) -> Self {
        self.with_loaded_builtin(load_from_bytes(wasm_bytes))
    }

    fn with_loaded_builtin(mut self, loaded: Result<(PluginDescriptor, Vec<u8>), String>) -> Self {
        match loaded {
            Ok((descriptor, bytes)) => {
                let descriptor = apply_builtin_indexer_overrides(descriptor);
                if !validate_indexer_descriptor(&descriptor, PluginLoadSource::Builtin) {
                    return self;
                }

                let provider_type = descriptor.provider_type().trim().to_ascii_lowercase();
                let aliases: Vec<String> = descriptor
                    .provider_aliases()
                    .iter()
                    .map(|a| a.trim().to_ascii_lowercase())
                    .collect();

                // Register primary provider_type (external overrides built-in)
                if self.plugins.contains_key(&provider_type) {
                    info!(
                        provider_type = provider_type.as_str(),
                        "external plugin overrides built-in"
                    );
                } else {
                    info!(
                        plugin = descriptor.name.as_str(),
                        version = descriptor.version.as_str(),
                        provider_type = provider_type.as_str(),
                        "registered built-in plugin"
                    );
                    self.plugins.insert(
                        provider_type.clone(),
                        LoadedPlugin {
                            wasm_bytes: bytes.clone(),
                            descriptor: descriptor.clone(),
                        },
                    );
                }

                // Register aliases (external overrides built-in)
                for alias in &aliases {
                    if self.plugins.contains_key(alias) {
                        info!(
                            alias = alias.as_str(),
                            provider_type = provider_type.as_str(),
                            "external plugin overrides built-in alias"
                        );
                    } else {
                        self.plugins.insert(
                            alias.clone(),
                            LoadedPlugin {
                                wasm_bytes: bytes.clone(),
                                descriptor: descriptor.clone(),
                            },
                        );
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to load built-in plugin");
            }
        }
        self
    }
}

fn apply_builtin_indexer_overrides(mut descriptor: PluginDescriptor) -> PluginDescriptor {
    if descriptor.provider_type().eq_ignore_ascii_case("nzbgeek") {
        descriptor.set_default_base_url(Some(NZBGEEK_DEFAULT_BASE_URL.to_string()));
    }
    if descriptor.provider_type().eq_ignore_ascii_case("dognzb") {
        descriptor.set_default_base_url(Some(DOGNZB_DEFAULT_BASE_URL.to_string()));
        descriptor
            .config_fields_mut()
            .retain(|field| field.key != "api_path" && field.key != "additional_params");
    }

    descriptor
}

impl IndexerPluginProvider for WasmIndexerPluginProvider {
    fn available_provider_types(&self) -> Vec<String> {
        // Only return primary provider_types, not aliases (which map to the same plugin)
        self.plugins
            .iter()
            .filter(|(key, loaded)| {
                **key
                    == loaded
                        .descriptor
                        .provider_type()
                        .trim()
                        .to_ascii_lowercase()
            })
            .map(|(key, _)| key.clone())
            .collect()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        builtin_indexer_provider_types()
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.version.clone())
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.sdk_version.clone())
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| plugin_descriptor_sdk_constraint(&loaded.descriptor))
    }

    fn plugin_type_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.plugin_type().to_string())
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        // Deduplicate: multiple keys may point to the same plugin. Use the
        // primary provider_type as the canonical source for scoring policies.
        let mut seen = std::collections::HashSet::new();
        self.plugins
            .values()
            .filter(|loaded| seen.insert(loaded.descriptor.provider_type().to_string()))
            .flat_map(|loaded| {
                loaded.descriptor.indexer().into_iter().flat_map(|indexer| {
                    indexer.scoring_policies.iter().map(|sp| {
                        // ID must be a valid Rego path segment (letters, digits, underscores).
                        let safe_provider = loaded
                            .descriptor
                            .provider_type()
                            .replace(['-', ':', '.'], "_");
                        let safe_name = sp.name.replace(['-', ':', '.'], "_");
                        let id = format!("plugin_{safe_provider}_{safe_name}");
                        scryer_rules::UserPolicy {
                            id,
                            name: sp.name.clone(),
                            rego_source: sp.rego_source.clone(),
                            applied_facets: sp.applied_facets.clone(),
                        }
                    })
                })
            })
            .collect()
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| config_fields_to_domain(loaded.descriptor.config_fields()))
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .and_then(|loaded| loaded.descriptor.default_base_url().map(ToOwned::to_owned))
    }

    fn rate_limit_seconds_for_provider(&self, provider_type: &str) -> Option<i64> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins.get(&key).and_then(|loaded| {
            loaded
                .descriptor
                .indexer()
                .and_then(|indexer| indexer.rate_limit_seconds)
        })
    }

    fn capabilities_for_provider(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerProviderCapabilities {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| {
                loaded
                    .descriptor
                    .indexer()
                    .map(|indexer| indexer_capabilities_to_domain(&indexer.capabilities))
                    .unwrap_or_default()
            })
            .unwrap_or(scryer_domain::IndexerProviderCapabilities {
                rss: true,
                supported_ids: std::collections::HashMap::from([
                    ("movie".into(), vec!["imdb_id".into()]),
                    ("series".into(), vec!["tvdb_id".into()]),
                ]),
                deduplicates_aliases: false,
                season_param: Some("season".into()),
                episode_param: Some("ep".into()),
                query_param: Some("q".into()),
                search: true,
                imdb_search: true,
                tvdb_search: true,
                anidb_search: false,
                ..Default::default()
            })
    }

    fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        let provider = config.provider_type.trim().to_ascii_lowercase();
        let loaded = self.plugins.get(&provider)?;

        match WasmIndexerClient::new(
            loaded.wasm_bytes.clone(),
            loaded.descriptor.clone(),
            config.name.clone(),
            config.clone(),
        ) {
            Ok(client) => Some(Arc::new(client)),
            Err(e) => {
                tracing::warn!(
                    indexer = config.name.as_str(),
                    provider = provider.as_str(),
                    error = %e,
                    "failed to compile WASM plugin, indexer will be unavailable"
                );
                None
            }
        }
    }
}

/// A thread-safe wrapper around `WasmIndexerPluginProvider` that supports
/// runtime reload. All reads acquire a `RwLock` read lock; `reload()` acquires
/// a write lock to swap the inner provider.
///
/// Caches instantiated `IndexerClient`s by `(indexer_config_id, updated_at)` so
/// WASM compilation only happens once per config revision. The cache is cleared
/// on provider reload.
pub struct DynamicPluginProvider {
    inner: std::sync::RwLock<WasmIndexerPluginProvider>,
    client_cache: std::sync::Mutex<HashMap<(String, String), Arc<dyn IndexerClient>>>,
}

impl DynamicPluginProvider {
    pub fn new(provider: WasmIndexerPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Replace the inner provider. This is called after install/uninstall/toggle.
    pub fn reload(&self, new_provider: WasmIndexerPluginProvider) {
        let mut guard = self
            .inner
            .write()
            .expect("DynamicPluginProvider lock poisoned");
        *guard = new_provider;
        // Clear the client cache — WASM bytes may have changed.
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.clear();
        }
        info!("plugin provider reloaded");
    }
}

impl IndexerPluginProvider for DynamicPluginProvider {
    fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        let cache_key = (config.id.clone(), config.updated_at.to_rfc3339());

        // Fast path: check cache first
        if let Ok(cache) = self.client_cache.lock()
            && let Some(client) = cache.get(&cache_key)
        {
            return Some(Arc::clone(client));
        }

        // Slow path: compile WASM and cache the result
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        let client = guard.client_for_provider(config)?;

        if let Ok(mut cache) = self.client_cache.lock() {
            cache.insert(cache_key, Arc::clone(&client));
        }

        Some(client)
    }

    fn available_provider_types(&self) -> Vec<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.available_provider_types()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        builtin_indexer_provider_types()
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.plugin_version_for_provider(provider_type)
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.plugin_sdk_version_for_provider(provider_type)
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.plugin_sdk_constraint_for_provider(provider_type)
    }

    fn plugin_type_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.plugin_type_for_provider(provider_type)
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.scoring_policies()
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.config_fields_for_provider(provider_type)
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.plugin_name_for_provider(provider_type)
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.default_base_url_for_provider(provider_type)
    }

    fn rate_limit_seconds_for_provider(&self, provider_type: &str) -> Option<i64> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.rate_limit_seconds_for_provider(provider_type)
    }

    fn capabilities_for_provider(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerProviderCapabilities {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.capabilities_for_provider(provider_type)
    }

    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_indexer_plugin_provider(
            external_wasm_bytes,
            disabled_builtins,
        ));
        Ok(())
    }
}

// ── Download client plugin provider ────────────────────────────────────

pub struct WasmDownloadClientPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
}

impl WasmDownloadClientPluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    pub fn with_external_bytes(self, wasm_bytes: &[u8]) -> Self {
        self.with_external_plugin(ExternalPluginWasm {
            bytes: wasm_bytes,
            first_party: false,
        })
    }

    fn with_external_plugin(mut self, plugin: ExternalPluginWasm<'_>) -> Self {
        match load_from_bytes(plugin.bytes) {
            Ok((descriptor, bytes)) => {
                if !validate_descriptor_for_type(
                    &descriptor,
                    Some("download_client"),
                    PluginLoadSource::External {
                        first_party: plugin.first_party,
                    },
                ) {
                    return self;
                }

                let provider_type = descriptor.provider_type().trim().to_ascii_lowercase();
                info!(
                    plugin = descriptor.name.as_str(),
                    version = descriptor.version.as_str(),
                    provider_type = provider_type.as_str(),
                    "registered external download client plugin"
                );
                self.plugins.insert(
                    provider_type,
                    LoadedPlugin {
                        wasm_bytes: bytes,
                        descriptor,
                    },
                );
            }
            Err(e) => {
                warn!(error = %e, "failed to load external download client plugin");
            }
        }
        self
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins.remove(&key);
        self
    }

    fn create_download_client(
        loaded: &LoadedPlugin,
        config: &DownloadClientConfig,
    ) -> Option<Arc<dyn DownloadClient>> {
        let mut manifest = Manifest::new([extism::Wasm::data(loaded.wasm_bytes.clone())]);
        let computed_base_url = compute_base_url_from_config_json(&config.config_json);
        manifest = apply_allowed_hosts(
            manifest,
            &loaded.descriptor,
            computed_base_url.as_deref(),
            Some(&config.config_json),
        );
        manifest = manifest.with_timeout(std::time::Duration::from_secs(30));

        if let Some(ref base_url) = computed_base_url {
            manifest = manifest.with_config_key("base_url", base_url);
        }

        match parse_config_json_entries(&config.config_json) {
            Ok(map) => {
                for (k, v) in &map {
                    manifest = manifest.with_config_key(k, v);
                }
            }
            Err(error) => {
                warn!(
                    client = config.name.as_str(),
                    error = %error,
                    "failed to parse download client config_json"
                );
            }
        }

        match build_plugin(manifest) {
            Ok(plugin) => {
                let client = WasmDownloadClient::new(
                    plugin,
                    loaded.descriptor.clone(),
                    config.id.clone(),
                    config.name.clone(),
                );
                Some(Arc::new(client))
            }
            Err(e) => {
                warn!(
                    client = config.name.as_str(),
                    provider_type = config.client_type.as_str(),
                    error = %e,
                    "failed to instantiate WASM download client plugin"
                );
                None
            }
        }
    }
}

impl DownloadClientPluginProvider for WasmDownloadClientPluginProvider {
    fn client_for_config(&self, config: &DownloadClientConfig) -> Option<Arc<dyn DownloadClient>> {
        let provider = config.client_type.trim().to_ascii_lowercase();
        let loaded = self.plugins.get(&provider)?;
        Self::create_download_client(loaded, config)
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.plugins
            .iter()
            .filter(|(key, loaded)| {
                **key
                    == loaded
                        .descriptor
                        .provider_type()
                        .trim()
                        .to_ascii_lowercase()
            })
            .map(|(key, _)| key.clone())
            .collect()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.version.clone())
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.sdk_version.clone())
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| plugin_descriptor_sdk_constraint(&loaded.descriptor))
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| config_fields_to_domain(loaded.descriptor.config_fields()))
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .and_then(|loaded| loaded.descriptor.default_base_url().map(ToOwned::to_owned))
    }

    fn accepted_inputs_for_provider(&self, provider_type: &str) -> Vec<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .and_then(|loaded| loaded.descriptor.download_client())
            .map(|download_client| {
                download_client
                    .accepted_inputs
                    .iter()
                    .map(|kind| serde_json::to_value(kind).unwrap_or_default())
                    .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn reload_plugins(
        &self,
        _external_wasm_bytes: &[ExternalPluginWasm<'_>],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        Err("use DynamicDownloadClientPluginProvider for reload".to_string())
    }
}

pub struct DynamicDownloadClientPluginProvider {
    inner: std::sync::RwLock<WasmDownloadClientPluginProvider>,
    client_cache: std::sync::Mutex<HashMap<(String, String), Arc<dyn DownloadClient>>>,
}

impl DynamicDownloadClientPluginProvider {
    pub fn new(provider: WasmDownloadClientPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn reload(&self, new_provider: WasmDownloadClientPluginProvider) {
        let mut guard = self
            .inner
            .write()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        *guard = new_provider;
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.clear();
        }
        info!("download client plugin provider reloaded");
    }
}

impl DownloadClientPluginProvider for DynamicDownloadClientPluginProvider {
    fn client_for_config(&self, config: &DownloadClientConfig) -> Option<Arc<dyn DownloadClient>> {
        let cache_key = (config.id.clone(), config.updated_at.to_rfc3339());

        if let Ok(cache) = self.client_cache.lock()
            && let Some(client) = cache.get(&cache_key)
        {
            return Some(Arc::clone(client));
        }

        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        let client = guard.client_for_config(config)?;

        if let Ok(mut cache) = self.client_cache.lock() {
            cache.insert(cache_key, Arc::clone(&client));
        }

        Some(client)
    }

    fn available_provider_types(&self) -> Vec<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.available_provider_types()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.plugin_version_for_provider(provider_type)
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.plugin_sdk_version_for_provider(provider_type)
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.plugin_sdk_constraint_for_provider(provider_type)
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.config_fields_for_provider(provider_type)
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.plugin_name_for_provider(provider_type)
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.default_base_url_for_provider(provider_type)
    }

    fn accepted_inputs_for_provider(&self, provider_type: &str) -> Vec<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.accepted_inputs_for_provider(provider_type)
    }

    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_download_client_plugin_provider(
            external_wasm_bytes,
            disabled_builtins,
        ));
        Ok(())
    }
}

/// Validate a plugin descriptor, optionally filtering by a specific plugin type.
/// If `expected_type` is None, any supported type passes.
fn validate_descriptor_for_type(
    descriptor: &PluginDescriptor,
    expected_type: Option<&str>,
    load_source: PluginLoadSource,
) -> bool {
    if let Err(error) = validate_plugin_descriptor_sdk_contract(descriptor, SDK_VERSION) {
        warn!(
            plugin = descriptor.name.as_str(),
            sdk_version = descriptor.sdk_version.as_str(),
            sdk_constraint = plugin_descriptor_sdk_constraint(descriptor),
            host_sdk_version = SDK_VERSION,
            error = error.as_str(),
            "skipping plugin: incompatible sdk contract"
        );
        return false;
    }

    if let Some(expected) = expected_type
        && descriptor.plugin_type() != expected
    {
        return false;
    }

    for host in descriptor.allowed_hosts() {
        if !allowed_host_pattern_is_valid(host) {
            warn!(
                plugin = descriptor.name.as_str(),
                provider_type = descriptor.provider_type(),
                host,
                "skipping plugin: invalid network permission pattern"
            );
            return false;
        }
    }

    let provider_matches_kind = matches!(
        (descriptor.kind(), &descriptor.provider),
        (PluginKind::Indexer, ProviderDescriptor::Indexer(_))
            | (
                PluginKind::Notification,
                ProviderDescriptor::Notification(_)
            )
            | (
                PluginKind::DownloadClient,
                ProviderDescriptor::DownloadClient(_)
            )
            | (
                PluginKind::SubtitleProvider,
                ProviderDescriptor::Subtitle(_)
            )
    );
    if !provider_matches_kind {
        warn!(
            plugin = descriptor.name.as_str(),
            plugin_type = descriptor.plugin_type(),
            "skipping plugin: descriptor kind and provider block do not match"
        );
        return false;
    }

    for field in descriptor.config_fields() {
        match field.value_source {
            ConfigFieldValueSource::User => {
                if field.host_binding.is_some() {
                    warn!(
                        plugin = descriptor.name.as_str(),
                        provider_type = descriptor.provider_type(),
                        field_key = field.key.as_str(),
                        "skipping plugin: user-sourced config field must not declare host_binding"
                    );
                    return false;
                }
            }
            ConfigFieldValueSource::HostBinding => {
                let Some(binding) = field.host_binding else {
                    warn!(
                        plugin = descriptor.name.as_str(),
                        provider_type = descriptor.provider_type(),
                        field_key = field.key.as_str(),
                        "skipping plugin: host-binding field must declare host_binding"
                    );
                    return false;
                };

                if !binding_allowed_for_plugin(binding, descriptor, load_source) {
                    warn!(
                        plugin = descriptor.name.as_str(),
                        provider_type = descriptor.provider_type(),
                        binding = binding.as_str(),
                        "skipping plugin: host_binding is not permitted for this plugin"
                    );
                    return false;
                }
            }
        }
    }

    true
}

fn is_indexer_plugin_type(plugin_type: &str) -> bool {
    INDEXER_PLUGIN_TYPES.contains(&plugin_type)
}

fn validate_indexer_descriptor(
    descriptor: &PluginDescriptor,
    load_source: PluginLoadSource,
) -> bool {
    validate_descriptor_for_type(descriptor, None, load_source)
        && is_indexer_plugin_type(descriptor.plugin_type())
}

fn binding_allowed_for_plugin(
    binding: SdkHostBinding,
    descriptor: &PluginDescriptor,
    load_source: PluginLoadSource,
) -> bool {
    match binding {
        SdkHostBinding::SmgOpenSubtitlesApiKey => {
            load_source.can_use_first_party_host_bindings()
                && descriptor.plugin_type() == "subtitle_provider"
                && descriptor.id.eq_ignore_ascii_case("opensubtitles")
                && descriptor
                    .provider_type()
                    .eq_ignore_ascii_case("opensubtitles")
        }
    }
}

fn allowed_host_pattern_is_valid(host: &str) -> bool {
    let host = host.trim();
    if host.is_empty()
        || host == "*"
        || host.contains("://")
        || host.contains('/')
        || host.contains('?')
        || host.contains('#')
        || host.contains(':')
    {
        return false;
    }

    if let Some(suffix) = host.strip_prefix("*.") {
        return !suffix.is_empty() && !suffix.contains('*') && url::Host::parse(suffix).is_ok();
    }

    !host.contains('*') && url::Host::parse(host).is_ok()
}

pub fn build_indexer_plugin_provider(
    external_wasm_bytes: &[ExternalPluginWasm<'_>],
    disabled_builtins: &[String],
) -> WasmIndexerPluginProvider {
    let mut provider = WasmIndexerPluginProvider::empty();

    for plugin in external_wasm_bytes {
        provider = provider.with_external_plugin(*plugin);
    }

    for loaded in load_builtin_bytes_parallel(&[
        crate::builtins::NZBGEEK_WASM,
        crate::builtins::NEWZNAB_WASM,
        crate::builtins::ANIMETOSHO_WASM,
        crate::builtins::TORZNAB_WASM,
    ]) {
        provider = provider.with_loaded_builtin(loaded);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

pub fn build_download_client_plugin_provider(
    external_wasm_bytes: &[ExternalPluginWasm<'_>],
    disabled_builtins: &[String],
) -> WasmDownloadClientPluginProvider {
    let mut provider = WasmDownloadClientPluginProvider::empty();

    for plugin in external_wasm_bytes {
        provider = provider.with_external_plugin(*plugin);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

// ── Subtitle provider plugin provider ────────────────────────────────

pub struct WasmSubtitlePluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
}

impl WasmSubtitlePluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    pub fn with_external_bytes(self, wasm_bytes: &[u8]) -> Self {
        self.with_external_plugin(ExternalPluginWasm {
            bytes: wasm_bytes,
            first_party: false,
        })
    }

    fn with_external_plugin(mut self, plugin: ExternalPluginWasm<'_>) -> Self {
        match load_from_bytes(plugin.bytes) {
            Ok((descriptor, bytes)) => {
                if !validate_descriptor_for_type(
                    &descriptor,
                    Some("subtitle_provider"),
                    PluginLoadSource::External {
                        first_party: plugin.first_party,
                    },
                ) {
                    return self;
                }

                let provider_type = descriptor.provider_type().trim().to_ascii_lowercase();
                info!(
                    plugin = descriptor.name.as_str(),
                    version = descriptor.version.as_str(),
                    provider_type = provider_type.as_str(),
                    "registered external subtitle provider plugin"
                );
                self.plugins.insert(
                    provider_type,
                    LoadedPlugin {
                        wasm_bytes: bytes,
                        descriptor,
                    },
                );
            }
            Err(error) => {
                warn!(error = %error, "failed to load external subtitle provider plugin");
            }
        }
        self
    }

    pub fn with_builtin(self, wasm_bytes: &[u8]) -> Self {
        self.with_loaded_builtin(load_from_bytes(wasm_bytes))
    }

    fn with_loaded_builtin(mut self, loaded: Result<(PluginDescriptor, Vec<u8>), String>) -> Self {
        match loaded {
            Ok((descriptor, bytes)) => {
                if !validate_descriptor_for_type(
                    &descriptor,
                    Some("subtitle_provider"),
                    PluginLoadSource::Builtin,
                ) {
                    return self;
                }

                let provider_type = descriptor.provider_type().trim().to_ascii_lowercase();
                if self.plugins.contains_key(&provider_type) {
                    return self;
                }

                info!(
                    plugin = descriptor.name.as_str(),
                    version = descriptor.version.as_str(),
                    provider_type = provider_type.as_str(),
                    "registered built-in subtitle provider plugin"
                );
                self.plugins.insert(
                    provider_type,
                    LoadedPlugin {
                        wasm_bytes: bytes,
                        descriptor,
                    },
                );
            }
            Err(error) => {
                warn!(error = %error, "failed to load built-in subtitle provider plugin");
            }
        }
        self
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins.remove(&key);
        self
    }
}

impl SubtitlePluginProvider for WasmSubtitlePluginProvider {
    fn client_for_config(
        &self,
        config: &SubtitleProviderConfig,
        host_bindings: &HashMap<PluginHostBindingId, String>,
    ) -> Option<Arc<dyn SubtitleProviderClient>> {
        let provider = config.provider_type.trim().to_ascii_lowercase();
        let loaded = self.plugins.get(&provider)?;
        match WasmSubtitleClient::new(
            loaded.wasm_bytes.clone(),
            loaded.descriptor.clone(),
            config.clone(),
            host_bindings.clone(),
        ) {
            Ok(client) => Some(Arc::new(client)),
            Err(error) => {
                warn!(
                    subtitle_provider = config.name.as_str(),
                    provider_type = provider.as_str(),
                    error = %error,
                    "failed to instantiate WASM subtitle provider plugin"
                );
                None
            }
        }
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.plugins
            .iter()
            .filter(|(key, loaded)| {
                **key
                    == loaded
                        .descriptor
                        .provider_type()
                        .trim()
                        .to_ascii_lowercase()
            })
            .map(|(key, _)| key.clone())
            .collect()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        builtin_subtitle_provider_types()
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.version.clone())
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.sdk_version.clone())
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| plugin_descriptor_sdk_constraint(&loaded.descriptor))
    }

    fn supports_catalog_search_for_provider(&self, provider_type: &str) -> bool {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .and_then(|loaded| loaded.descriptor.subtitle())
            .is_some_and(|subtitle| subtitle.capabilities.mode == SubtitleProviderMode::Catalog)
    }

    fn recommended_facets_for_provider(&self, provider_type: &str) -> Vec<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .and_then(|loaded| loaded.descriptor.subtitle())
            .map(|subtitle| subtitle.capabilities.recommended_facets.clone())
            .unwrap_or_default()
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| config_fields_to_domain(loaded.descriptor.config_fields()))
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn reload_plugins(
        &self,
        _external_wasm_bytes: &[ExternalPluginWasm<'_>],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        Err("use DynamicSubtitlePluginProvider for reload".to_string())
    }
}

pub struct DynamicSubtitlePluginProvider {
    inner: std::sync::RwLock<WasmSubtitlePluginProvider>,
    client_cache: SubtitleClientCache,
}

impl DynamicSubtitlePluginProvider {
    pub fn new(provider: WasmSubtitlePluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn reload(&self, new_provider: WasmSubtitlePluginProvider) {
        let mut guard = self
            .inner
            .write()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        *guard = new_provider;
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.clear();
        }
        info!("subtitle plugin provider reloaded");
    }
}

impl SubtitlePluginProvider for DynamicSubtitlePluginProvider {
    fn client_for_config(
        &self,
        config: &SubtitleProviderConfig,
        host_bindings: &HashMap<PluginHostBindingId, String>,
    ) -> Option<Arc<dyn SubtitleProviderClient>> {
        let cache_key = (
            config.id.clone(),
            config.updated_at.to_rfc3339(),
            cache_fingerprint(&config.config_json),
            host_binding_cache_key(host_bindings),
        );

        if let Ok(cache) = self.client_cache.lock()
            && let Some(client) = cache.get(&cache_key)
        {
            return Some(Arc::clone(client));
        }

        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        let client = guard.client_for_config(config, host_bindings)?;

        if let Ok(mut cache) = self.client_cache.lock() {
            cache.insert(cache_key, Arc::clone(&client));
        }

        Some(client)
    }

    fn available_provider_types(&self) -> Vec<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.available_provider_types()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        builtin_subtitle_provider_types()
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.plugin_version_for_provider(provider_type)
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.plugin_sdk_version_for_provider(provider_type)
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.plugin_sdk_constraint_for_provider(provider_type)
    }

    fn supports_catalog_search_for_provider(&self, provider_type: &str) -> bool {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.supports_catalog_search_for_provider(provider_type)
    }

    fn recommended_facets_for_provider(&self, provider_type: &str) -> Vec<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.recommended_facets_for_provider(provider_type)
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.config_fields_for_provider(provider_type)
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.plugin_name_for_provider(provider_type)
    }

    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_subtitle_plugin_provider(
            external_wasm_bytes,
            disabled_builtins,
        ));
        Ok(())
    }
}

fn host_binding_cache_key(host_bindings: &HashMap<PluginHostBindingId, String>) -> String {
    let mut entries = host_bindings
        .iter()
        .map(|(binding, value)| (binding.as_str(), value))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(right.0));
    cache_fingerprint(
        &entries
            .into_iter()
            .map(|(binding, value)| format!("{binding}={value}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn cache_fingerprint(value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn build_subtitle_plugin_provider(
    external_wasm_bytes: &[ExternalPluginWasm<'_>],
    disabled_builtins: &[String],
) -> WasmSubtitlePluginProvider {
    let mut provider = WasmSubtitlePluginProvider::empty();

    for plugin in external_wasm_bytes {
        provider = provider.with_external_plugin(*plugin);
    }

    for loaded in load_builtin_bytes_parallel(&[crate::builtins::JIMAKU_WASM]) {
        provider = provider.with_loaded_builtin(loaded);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

/// Scan `plugins_dir` for subdirectories containing `plugin.wasm`, load each,
/// call `describe()` to get the plugin descriptor, and return a provider that
/// can create indexer clients for any loaded plugin type.
pub fn load_indexer_plugins(plugins_dir: &Path) -> Result<WasmIndexerPluginProvider, String> {
    let mut plugins = HashMap::new();

    let entries = std::fs::read_dir(plugins_dir).map_err(|e| {
        format!(
            "failed to read plugins directory {}: {e}",
            plugins_dir.display()
        )
    })?;

    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }

        let wasm_path = dir.join("plugin.wasm");
        if !wasm_path.exists() {
            continue;
        }

        match load_single_plugin(&wasm_path) {
            Ok((descriptor, wasm_bytes)) => {
                if !validate_indexer_descriptor(
                    &descriptor,
                    PluginLoadSource::External { first_party: false },
                ) {
                    continue;
                }

                let provider_type = descriptor.provider_type().trim().to_ascii_lowercase();

                // Check for duplicates
                if plugins.contains_key(&provider_type) {
                    warn!(
                        plugin = descriptor.name.as_str(),
                        provider_type = provider_type.as_str(),
                        "skipping plugin: duplicate provider_type already loaded"
                    );
                    continue;
                }

                info!(
                    plugin = descriptor.name.as_str(),
                    version = descriptor.version.as_str(),
                    provider_type = provider_type.as_str(),
                    "loaded indexer plugin"
                );

                // Register aliases
                let aliases: Vec<String> = descriptor
                    .provider_aliases()
                    .iter()
                    .map(|a| a.trim().to_ascii_lowercase())
                    .collect();
                for alias in &aliases {
                    if !plugins.contains_key(alias) {
                        plugins.insert(
                            alias.clone(),
                            LoadedPlugin {
                                wasm_bytes: wasm_bytes.clone(),
                                descriptor: descriptor.clone(),
                            },
                        );
                    }
                }

                plugins.insert(
                    provider_type,
                    LoadedPlugin {
                        wasm_bytes,
                        descriptor,
                    },
                );
            }
            Err(e) => {
                warn!(
                    path = %wasm_path.display(),
                    error = %e,
                    "failed to load plugin"
                );
            }
        }
    }

    Ok(WasmIndexerPluginProvider { plugins })
}

fn load_single_plugin(wasm_path: &Path) -> Result<(PluginDescriptor, Vec<u8>), String> {
    let wasm_bytes = std::fs::read(wasm_path)
        .map_err(|e| format!("failed to read {}: {e}", wasm_path.display()))?;

    load_from_bytes(&wasm_bytes)
}

pub(crate) fn parse_config_json_entries(json_str: &str) -> Result<HashMap<String, String>, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(json_str).map_err(|error| error.to_string())?;
    let object = parsed
        .as_object()
        .ok_or_else(|| "config_json must be a JSON object".to_string())?;

    let mut entries = HashMap::with_capacity(object.len());
    for (key, value) in object {
        if value.is_null() {
            continue;
        }

        let normalized = match value {
            serde_json::Value::String(value) => value.clone(),
            other => other.to_string(),
        };
        entries.insert(key.clone(), normalized);
    }

    Ok(entries)
}

/// Compute a base URL from host/port/use_ssl/url_base in config_json.
fn compute_base_url_from_config_json(json_str: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let host = parsed
        .get("host")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?;
    let port = parsed.get("port").and_then(|v| match v {
        serde_json::Value::String(s) => Some(s.as_str().to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    });
    let use_ssl = parsed
        .get("use_ssl")
        .or_else(|| parsed.get("useSsl"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let url_base = parsed
        .get("url_base")
        .or_else(|| parsed.get("urlBase"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

    let protocol = if use_ssl { "https" } else { "http" };
    let mut url = format!("{protocol}://{host}");
    if let Some(p) = port.filter(|p| !p.is_empty()) {
        url.push(':');
        url.push_str(&p);
    }
    if let Some(base) = url_base {
        let normalized = base.trim_start_matches('/');
        if !normalized.is_empty() {
            url.push('/');
            url.push_str(normalized);
        }
    }
    Some(url)
}

/// Build the Extism allowed-hosts list for a plugin manifest.
///
/// The allowed hosts are derived from:
/// 1. The plugin's `allowed_hosts` descriptor field (static declarations).
/// 2. The hostname from `base_url` (indexer plugins).
/// 3. Hostnames from `config_json` values that parse as URLs (notification plugins).
///
/// If the resulting set is empty, no hosts are allowed (plugin has no network access).
pub(crate) fn apply_allowed_hosts(
    mut manifest: Manifest,
    descriptor: &PluginDescriptor,
    base_url: Option<&str>,
    config_json: Option<&str>,
) -> Manifest {
    let mut hosts: Vec<String> = descriptor.allowed_hosts().to_vec();

    // Add hostname from base_url (indexer plugins)
    if let Some(url_str) = base_url
        && let Some(host) = host_from_url(url_str)
    {
        hosts.push(host);
    }

    // Add hostnames from config_json values that parse as URLs (notification plugins)
    if let Some(json_str) = config_json
        && let Ok(map) = parse_config_json_entries(json_str)
    {
        for value in map.values() {
            if let Some(host) = host_from_url(value) {
                hosts.push(host);
            }
        }
    }

    for host in &hosts {
        manifest = manifest.with_allowed_host(host);
    }
    manifest
}

fn host_from_url(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(ToOwned::to_owned))
}

pub(crate) fn build_plugin(manifest: Manifest) -> Result<extism::Plugin, extism::Error> {
    // Wasmtime's filesystem cache is not race-free when multiple identical
    // modules compile concurrently in the same process. Serialize the build
    // step so parallel provider/client loading does not emit cache rename
    // warnings for a benign same-artifact race.
    let _build_guard = WASMTIME_PLUGIN_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    extism::PluginBuilder::new(manifest)
        .with_wasi(true)
        .with_http_response_headers(true)
        .build()
}

fn required_exports_for_descriptor(descriptor: &PluginDescriptor) -> Vec<&'static str> {
    let mut exports = vec![EXPORT_DESCRIBE];
    match &descriptor.provider {
        ProviderDescriptor::Indexer(_) => {
            exports.push(EXPORT_INDEXER_SEARCH);
        }
        ProviderDescriptor::DownloadClient(_) => {
            exports.extend([
                EXPORT_DOWNLOAD_ADD,
                EXPORT_DOWNLOAD_LIST_QUEUE,
                EXPORT_DOWNLOAD_LIST_HISTORY,
                EXPORT_DOWNLOAD_LIST_COMPLETED,
                EXPORT_DOWNLOAD_CONTROL,
                EXPORT_DOWNLOAD_MARK_IMPORTED,
                EXPORT_DOWNLOAD_STATUS,
                EXPORT_DOWNLOAD_TEST_CONNECTION,
            ]);
        }
        ProviderDescriptor::Notification(_) => {
            exports.push(EXPORT_NOTIFICATION_SEND);
        }
        ProviderDescriptor::Subtitle(subtitle) => {
            exports.push(EXPORT_VALIDATE_CONFIG);
            match subtitle.capabilities.mode {
                SubtitleProviderMode::Catalog => {
                    exports.extend([EXPORT_SUBTITLE_SEARCH, EXPORT_SUBTITLE_DOWNLOAD]);
                }
                SubtitleProviderMode::Generator => {
                    exports.push(EXPORT_SUBTITLE_GENERATE);
                }
            }
        }
    }
    exports
}

fn validate_required_exports(
    plugin: &extism::Plugin,
    descriptor: &PluginDescriptor,
) -> Result<(), String> {
    let missing = required_exports_for_descriptor(descriptor)
        .into_iter()
        .filter(|export| !plugin.function_exists(export))
        .collect::<Vec<_>>();

    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} ({}) is missing required export(s): {}",
            descriptor.id,
            descriptor.plugin_type(),
            missing.join(", ")
        ))
    }
}

fn load_from_bytes(wasm_bytes: &[u8]) -> Result<(PluginDescriptor, Vec<u8>), String> {
    let bytes = wasm_bytes.to_vec();
    // No allowed hosts needed — describe() is a pure function that returns JSON.
    let manifest = Manifest::new([extism::Wasm::data(bytes.clone())])
        .with_timeout(std::time::Duration::from_secs(10));

    let mut plugin =
        build_plugin(manifest).map_err(|e| format!("failed to instantiate WASM: {e}"))?;

    let output: String = plugin
        .call::<&str, String>(EXPORT_DESCRIBE, "")
        .map_err(|e| format!("{EXPORT_DESCRIBE}() failed: {e}"))?;

    let descriptor: PluginDescriptor = serde_json::from_str(&output)
        .map_err(|e| format!("describe() returned invalid JSON: {e}"))?;

    validate_required_exports(&plugin, &descriptor)?;

    Ok((descriptor, bytes))
}

fn load_builtin_bytes_parallel(
    wasm_bytes: &[&'static [u8]],
) -> Vec<Result<(PluginDescriptor, Vec<u8>), String>> {
    let handles = wasm_bytes
        .iter()
        .copied()
        .map(|bytes| std::thread::spawn(move || load_from_bytes(bytes)))
        .collect::<Vec<_>>();

    handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .unwrap_or_else(|_| Err("built-in plugin loader panicked".to_string()))
        })
        .collect()
}

// ── Notification plugin provider ───────────────────────────────────────

pub struct WasmNotificationPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
}

impl WasmNotificationPluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    pub fn with_external_bytes(self, wasm_bytes: &[u8]) -> Self {
        self.with_external_plugin(ExternalPluginWasm {
            bytes: wasm_bytes,
            first_party: false,
        })
    }

    fn with_external_plugin(mut self, plugin: ExternalPluginWasm<'_>) -> Self {
        match load_from_bytes(plugin.bytes) {
            Ok((descriptor, bytes)) => {
                if !validate_descriptor_for_type(
                    &descriptor,
                    Some("notification"),
                    PluginLoadSource::External {
                        first_party: plugin.first_party,
                    },
                ) {
                    return self;
                }

                let provider_type = descriptor.provider_type().trim().to_ascii_lowercase();
                info!(
                    plugin = descriptor.name.as_str(),
                    version = descriptor.version.as_str(),
                    provider_type = provider_type.as_str(),
                    "registered external notification plugin"
                );
                self.plugins.insert(
                    provider_type,
                    LoadedPlugin {
                        wasm_bytes: bytes,
                        descriptor,
                    },
                );
            }
            Err(e) => {
                warn!(error = %e, "failed to load external notification plugin");
            }
        }
        self
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins.remove(&key);
        self
    }

    fn create_notification_client(
        loaded: &LoadedPlugin,
        config: &NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>> {
        let mut manifest = Manifest::new([extism::Wasm::data(loaded.wasm_bytes.clone())]);
        manifest = apply_allowed_hosts(
            manifest,
            &loaded.descriptor,
            None,
            Some(&config.config_json),
        );
        manifest = manifest.with_timeout(std::time::Duration::from_secs(30));

        // Inject config_json key-value pairs
        match parse_config_json_entries(&config.config_json) {
            Ok(map) => {
                for (k, v) in &map {
                    manifest = manifest.with_config_key(k, v);
                }
            }
            Err(error) => {
                warn!(
                    channel = config.name.as_str(),
                    error = %error,
                    "failed to parse notification channel config_json"
                );
            }
        }

        match build_plugin(manifest) {
            Ok(plugin) => {
                let client = WasmNotificationClient::new(
                    plugin,
                    loaded.descriptor.clone(),
                    config.name.clone(),
                );
                Some(Arc::new(client))
            }
            Err(e) => {
                warn!(
                    channel = config.name.as_str(),
                    error = %e,
                    "failed to instantiate WASM notification plugin"
                );
                None
            }
        }
    }
}

impl NotificationPluginProvider for WasmNotificationPluginProvider {
    fn client_for_channel(
        &self,
        config: &NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>> {
        let provider = config.channel_type.as_str().to_ascii_lowercase();
        let loaded = self.plugins.get(&provider)?;
        Self::create_notification_client(loaded, config)
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.plugins
            .iter()
            .filter(|(key, loaded)| {
                **key
                    == loaded
                        .descriptor
                        .provider_type()
                        .trim()
                        .to_ascii_lowercase()
            })
            .map(|(key, _)| key.clone())
            .collect()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.version.clone())
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.sdk_version.clone())
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| plugin_descriptor_sdk_constraint(&loaded.descriptor))
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| config_fields_to_domain(loaded.descriptor.config_fields()))
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let key = provider_type.trim().to_ascii_lowercase();
        self.plugins
            .get(&key)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn reload_plugins(
        &self,
        _external_wasm_bytes: &[ExternalPluginWasm<'_>],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        Err("use DynamicNotificationPluginProvider for reload".to_string())
    }
}

/// Thread-safe wrapper around `WasmNotificationPluginProvider` that supports runtime reload.
pub struct DynamicNotificationPluginProvider {
    inner: std::sync::RwLock<WasmNotificationPluginProvider>,
    client_cache: std::sync::Mutex<HashMap<(String, String), Arc<dyn NotificationClient>>>,
}

impl DynamicNotificationPluginProvider {
    pub fn new(provider: WasmNotificationPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn reload(&self, new_provider: WasmNotificationPluginProvider) {
        let mut guard = self
            .inner
            .write()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        *guard = new_provider;
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.clear();
        }
        info!("notification plugin provider reloaded");
    }
}

impl NotificationPluginProvider for DynamicNotificationPluginProvider {
    fn client_for_channel(
        &self,
        config: &NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>> {
        let cache_key = (config.id.clone(), config.updated_at.to_rfc3339());

        if let Ok(cache) = self.client_cache.lock()
            && let Some(client) = cache.get(&cache_key)
        {
            return Some(Arc::clone(client));
        }

        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        let client = guard.client_for_channel(config)?;

        if let Ok(mut cache) = self.client_cache.lock() {
            cache.insert(cache_key, Arc::clone(&client));
        }

        Some(client)
    }

    fn available_provider_types(&self) -> Vec<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.available_provider_types()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.plugin_version_for_provider(provider_type)
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.plugin_sdk_version_for_provider(provider_type)
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.plugin_sdk_constraint_for_provider(provider_type)
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.config_fields_for_provider(provider_type)
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.plugin_name_for_provider(provider_type)
    }

    fn reload_plugins(
        &self,
        external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_notification_plugin_provider(
            external_wasm_bytes,
            disabled_builtins,
        ));
        Ok(())
    }
}

pub fn build_notification_plugin_provider(
    external_wasm_bytes: &[ExternalPluginWasm<'_>],
    disabled_builtins: &[String],
) -> WasmNotificationPluginProvider {
    let mut provider = WasmNotificationPluginProvider::empty();

    for plugin in external_wasm_bytes {
        provider = provider.with_external_plugin(*plugin);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ConfigFieldDef, ConfigFieldType, ConfigFieldValueSource, DownloadClientCapabilities,
        DownloadClientDescriptor, IndexerDescriptor, IndexerSourceKind, NotificationCapabilities,
        NotificationDescriptor, PluginHostBindingId, SubtitleCapabilities, SubtitleDescriptor,
    };

    fn descriptor(plugin_type: &str) -> PluginDescriptor {
        let provider = match plugin_type {
            "indexer" => ProviderDescriptor::Indexer(IndexerDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                source_kind: IndexerSourceKind::Generic,
                capabilities: Default::default(),
                scoring_policies: vec![],
                config_fields: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                rate_limit_seconds: None,
            }),
            "usenet_indexer" => ProviderDescriptor::Indexer(IndexerDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                source_kind: IndexerSourceKind::Usenet,
                capabilities: Default::default(),
                scoring_policies: vec![],
                config_fields: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                rate_limit_seconds: None,
            }),
            "torrent_indexer" => ProviderDescriptor::Indexer(IndexerDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                source_kind: IndexerSourceKind::Torrent,
                capabilities: Default::default(),
                scoring_policies: vec![],
                config_fields: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                rate_limit_seconds: None,
            }),
            "notification" => ProviderDescriptor::Notification(NotificationDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                config_fields: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                capabilities: NotificationCapabilities::default(),
            }),
            "download_client" => ProviderDescriptor::DownloadClient(DownloadClientDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                config_fields: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                accepted_inputs: vec![],
                isolation_modes: vec![],
                capabilities: DownloadClientCapabilities::default(),
            }),
            "subtitle_provider" => ProviderDescriptor::Subtitle(SubtitleDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                config_fields: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                capabilities: SubtitleCapabilities::default(),
            }),
            other => panic!("unsupported test plugin type: {other}"),
        };

        PluginDescriptor {
            id: "test".to_string(),
            name: "Test".to_string(),
            version: "0.1.0".to_string(),
            sdk_version: "1.0.0".to_string(),
            sdk_constraint: ">=1.0.0, <2.0.0".to_string(),
            provider,
        }
    }

    fn set_provider_type(descriptor: &mut PluginDescriptor, provider_type: &str) {
        match &mut descriptor.provider {
            ProviderDescriptor::Indexer(provider) => {
                provider.provider_type = provider_type.to_string()
            }
            ProviderDescriptor::Notification(provider) => {
                provider.provider_type = provider_type.to_string()
            }
            ProviderDescriptor::DownloadClient(provider) => {
                provider.provider_type = provider_type.to_string()
            }
            ProviderDescriptor::Subtitle(provider) => {
                provider.provider_type = provider_type.to_string()
            }
        }
    }

    fn set_allowed_hosts(descriptor: &mut PluginDescriptor, allowed_hosts: Vec<String>) {
        match &mut descriptor.provider {
            ProviderDescriptor::Indexer(provider) => provider.allowed_hosts = allowed_hosts,
            ProviderDescriptor::Notification(provider) => provider.allowed_hosts = allowed_hosts,
            ProviderDescriptor::DownloadClient(provider) => provider.allowed_hosts = allowed_hosts,
            ProviderDescriptor::Subtitle(provider) => provider.allowed_hosts = allowed_hosts,
        }
    }

    #[test]
    fn indexer_family_types_are_accepted() {
        assert!(validate_indexer_descriptor(
            &descriptor("indexer"),
            PluginLoadSource::External { first_party: false }
        ));
        assert!(validate_indexer_descriptor(
            &descriptor("usenet_indexer"),
            PluginLoadSource::External { first_party: false }
        ));
        assert!(validate_indexer_descriptor(
            &descriptor("torrent_indexer"),
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn non_indexer_types_are_rejected_for_indexer_provider() {
        assert!(!validate_indexer_descriptor(
            &descriptor("notification"),
            PluginLoadSource::External { first_party: false }
        ));
        assert!(!validate_indexer_descriptor(
            &descriptor("download_client"),
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn provider_type_collision_is_allowed_across_plugin_families() {
        let mut indexer = descriptor("indexer");
        set_provider_type(&mut indexer, "animetosho");

        let mut subtitle = descriptor("subtitle_provider");
        set_provider_type(&mut subtitle, "animetosho");

        assert!(validate_indexer_descriptor(
            &indexer,
            PluginLoadSource::External { first_party: false }
        ));
        assert!(validate_descriptor_for_type(
            &subtitle,
            Some("subtitle_provider"),
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn subtitle_provider_rejects_notification_expected_type() {
        let descriptor = descriptor("subtitle_provider");
        assert!(!validate_descriptor_for_type(
            &descriptor,
            Some("notification"),
            PluginLoadSource::Builtin
        ));
    }

    #[test]
    fn constrained_allowed_host_glob_is_accepted() {
        let mut descriptor = descriptor("subtitle_provider");
        set_allowed_hosts(&mut descriptor, vec!["*.opensubtitles.com".to_string()]);

        assert!(validate_descriptor_for_type(
            &descriptor,
            Some("subtitle_provider"),
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn malformed_allowed_host_patterns_are_rejected() {
        for pattern in [
            "*",
            "http://*.example.com",
            "*.*.example.com",
            "foo*bar.com",
            "example.com/path",
            "example.com:443",
        ] {
            let mut descriptor = descriptor("subtitle_provider");
            set_allowed_hosts(&mut descriptor, vec![pattern.to_string()]);
            assert!(
                !validate_descriptor_for_type(
                    &descriptor,
                    Some("subtitle_provider"),
                    PluginLoadSource::External { first_party: false }
                ),
                "pattern should be rejected: {pattern}"
            );
        }
    }

    #[test]
    fn official_external_opensubtitles_plugin_may_request_api_key_binding() {
        let mut descriptor = descriptor("subtitle_provider");
        descriptor.id = "opensubtitles".to_string();
        set_provider_type(&mut descriptor, "opensubtitles");
        let ProviderDescriptor::Subtitle(subtitle) = &mut descriptor.provider else {
            panic!("expected subtitle descriptor");
        };
        subtitle.config_fields = vec![ConfigFieldDef {
            key: "api_key".to_string(),
            label: "API Key".to_string(),
            field_type: ConfigFieldType::Password,
            required: true,
            default_value: None,
            value_source: ConfigFieldValueSource::HostBinding,
            host_binding: Some(PluginHostBindingId::SmgOpenSubtitlesApiKey),
            options: vec![],
            help_text: None,
        }];

        assert!(validate_descriptor_for_type(
            &descriptor,
            Some("subtitle_provider"),
            PluginLoadSource::External { first_party: true }
        ));
    }

    #[test]
    fn non_official_external_plugins_cannot_request_opensubtitles_api_key_binding() {
        let mut descriptor = descriptor("subtitle_provider");
        descriptor.id = "opensubtitles".to_string();
        set_provider_type(&mut descriptor, "opensubtitles");
        let ProviderDescriptor::Subtitle(subtitle) = &mut descriptor.provider else {
            panic!("expected subtitle descriptor");
        };
        subtitle.config_fields = vec![ConfigFieldDef {
            key: "api_key".to_string(),
            label: "API Key".to_string(),
            field_type: ConfigFieldType::Password,
            required: true,
            default_value: None,
            value_source: ConfigFieldValueSource::HostBinding,
            host_binding: Some(PluginHostBindingId::SmgOpenSubtitlesApiKey),
            options: vec![],
            help_text: None,
        }];

        assert!(!validate_descriptor_for_type(
            &descriptor,
            Some("subtitle_provider"),
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn non_subtitle_plugins_cannot_request_subtitle_host_bindings() {
        let mut descriptor = descriptor("notification");
        let ProviderDescriptor::Notification(notification) = &mut descriptor.provider else {
            panic!("expected notification descriptor");
        };
        notification.config_fields = vec![ConfigFieldDef {
            key: "api_key".to_string(),
            label: "API Key".to_string(),
            field_type: ConfigFieldType::Password,
            required: true,
            default_value: None,
            value_source: ConfigFieldValueSource::HostBinding,
            host_binding: Some(PluginHostBindingId::SmgOpenSubtitlesApiKey),
            options: vec![],
            help_text: None,
        }];

        assert!(!validate_descriptor_for_type(
            &descriptor,
            Some("notification"),
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn parse_config_json_entries_stringifies_scalar_values() {
        let entries = parse_config_json_entries(
            r#"{"username":"alice","password":"secret","use_ssl":false,"port":8080,"meta":{"tag":"series"}}"#,
        )
        .unwrap();

        assert_eq!(entries.get("username"), Some(&"alice".to_string()));
        assert_eq!(entries.get("password"), Some(&"secret".to_string()));
        assert_eq!(entries.get("use_ssl"), Some(&"false".to_string()));
        assert_eq!(entries.get("port"), Some(&"8080".to_string()));
        assert_eq!(
            entries.get("meta"),
            Some(&r#"{"tag":"series"}"#.to_string())
        );
    }

    #[test]
    fn parse_config_json_entries_requires_object_root() {
        let error = parse_config_json_entries(r#"["not","an","object"]"#).unwrap_err();
        assert_eq!(error, "config_json must be a JSON object");
    }

    #[test]
    fn subtitle_client_cache_fingerprint_changes_with_config_json() {
        assert_ne!(
            cache_fingerprint(r#"{"username":"alice"}"#),
            cache_fingerprint(r#"{"username":"bob"}"#)
        );
    }
}
