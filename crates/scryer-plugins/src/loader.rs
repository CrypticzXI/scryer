use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use extism::Manifest;
use scryer_application::{
    AppError, AppResult, AudioTranscoderClient, DownloadClient, DownloadClientPluginProvider,
    ExternalPluginWasm, IndexerClient, IndexerPluginProvider, NotificationClient,
    NotificationPluginProvider, PluginDescriptorLoader, RuntimePluginLoad, SubtitlePluginProvider,
    SubtitleProviderClient,
};
use scryer_domain::{
    DownloadClientConfig, IndexerConfig, NotificationChannelConfig, PluginHostBindingId,
    SubtitleProviderConfig,
};
use tracing::{info, warn};

use crate::audio_transcoder_adapter::WasmAudioTranscoderClient;
use crate::download_client_adapter::WasmDownloadClient;
use crate::indexer_adapter::WasmIndexerClient;
use crate::notification_adapter::WasmNotificationClient;
use crate::plugin_http_host;
use crate::socket_host::SocketHost;
use crate::subtitle_adapter::WasmSubtitleClient;
use crate::types::{
    ConfigFieldDef, ConfigFieldRole, ConfigFieldType, ConfigFieldValueSource, EXPORT_DESCRIBE,
    EXPORT_DOWNLOAD_ADD, EXPORT_DOWNLOAD_CONTROL, EXPORT_DOWNLOAD_LIST_COMPLETED,
    EXPORT_DOWNLOAD_LIST_HISTORY, EXPORT_DOWNLOAD_LIST_QUEUE, EXPORT_DOWNLOAD_MARK_IMPORTED,
    EXPORT_DOWNLOAD_STATUS, EXPORT_DOWNLOAD_TEST_CONNECTION, EXPORT_INDEXER_SEARCH,
    EXPORT_NOTIFICATION_SEND, EXPORT_SUBTITLE_DOWNLOAD, EXPORT_SUBTITLE_GENERATE,
    EXPORT_SUBTITLE_SEARCH, EXPORT_VALIDATE_CONFIG, PluginDescriptor,
    PluginHostBindingId as SdkHostBinding, PluginKind, ProviderDescriptor, SDK_VERSION,
    SubtitleProviderMode, config_fields_to_domain, indexer_capabilities_to_domain,
    plugin_descriptor_sdk_constraint, validate_plugin_descriptor_sdk_contract,
};

const NZBGEEK_DEFAULT_BASE_URL: &str = "https://api.nzbgeek.info";
const DOGNZB_DEFAULT_BASE_URL: &str = "https://api.dognzb.cr";
const INDEXER_PLUGIN_TYPES: &[&str] = &["indexer", "usenet_indexer", "torrent_indexer"];

type IndexerClientCacheKey = (String, String, String);
type IndexerClientCache = std::sync::Mutex<HashMap<IndexerClientCacheKey, Arc<dyn IndexerClient>>>;
type DownloadClientCacheKey = (String, String, String);
type DownloadClientCache =
    std::sync::Mutex<HashMap<DownloadClientCacheKey, Arc<dyn DownloadClient>>>;
type NotificationClientCacheKey = (String, String, String);
type NotificationClientCache =
    std::sync::Mutex<HashMap<NotificationClientCacheKey, Arc<dyn NotificationClient>>>;
type SubtitleClientCacheKey = (String, String, String, String, String);
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

enum LoadedPluginBacking {
    Owned(Vec<u8>),
    Builtin(crate::builtins::BuiltinPluginAsset),
}

struct LoadedPlugin {
    wasm: LoadedPluginBacking,
    descriptor: PluginDescriptor,
}

impl LoadedPlugin {
    fn from_owned(descriptor: PluginDescriptor, wasm_bytes: Vec<u8>) -> Self {
        Self {
            wasm: LoadedPluginBacking::Owned(wasm_bytes),
            descriptor,
        }
    }

    fn from_builtin(
        descriptor: PluginDescriptor,
        asset: crate::builtins::BuiltinPluginAsset,
    ) -> Self {
        Self {
            wasm: LoadedPluginBacking::Builtin(asset),
            descriptor,
        }
    }

    fn materialize_wasm(&self) -> Result<Vec<u8>, String> {
        match &self.wasm {
            LoadedPluginBacking::Owned(wasm_bytes) => Ok(wasm_bytes.clone()),
            LoadedPluginBacking::Builtin(asset) => crate::builtins::decode_builtin_wasm(*asset),
        }
    }

    #[cfg(test)]
    fn stores_builtin_asset(&self) -> bool {
        matches!(self.wasm, LoadedPluginBacking::Builtin(_))
    }
}

struct LoadedPluginRecord {
    primary_key: String,
    alias_keys: Vec<String>,
    loaded: LoadedPlugin,
}

impl LoadedPluginRecord {
    fn new(loaded: LoadedPlugin) -> Self {
        let primary_key = loaded
            .descriptor
            .provider_type()
            .trim()
            .to_ascii_lowercase();
        let alias_keys = loaded
            .descriptor
            .provider_aliases()
            .iter()
            .map(|alias| alias.trim().to_ascii_lowercase())
            .filter(|alias| !alias.is_empty() && alias != &primary_key)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        Self {
            primary_key,
            alias_keys,
            loaded,
        }
    }
}

fn resolve_loaded_plugin<'a>(
    plugins: &'a HashMap<String, LoadedPlugin>,
    aliases: &HashMap<String, String>,
    provider_type: &str,
) -> Option<&'a LoadedPlugin> {
    let key = provider_type.trim().to_ascii_lowercase();
    let primary = aliases
        .get(&key)
        .map(String::as_str)
        .unwrap_or(key.as_str());
    plugins.get(primary)
}

fn remove_loaded_plugin(
    plugins: &mut HashMap<String, LoadedPlugin>,
    aliases: &mut HashMap<String, String>,
    provider_type: &str,
) -> Vec<String> {
    let key = provider_type.trim().to_ascii_lowercase();
    let primary = aliases.get(&key).cloned().unwrap_or(key);
    let Some(_) = plugins.remove(&primary) else {
        return Vec::new();
    };

    let removed_aliases = aliases
        .iter()
        .filter(|(_, owner)| **owner == primary)
        .map(|(alias, _)| alias.clone())
        .collect::<Vec<_>>();
    for alias in &removed_aliases {
        aliases.remove(alias);
    }

    let mut affected = Vec::with_capacity(removed_aliases.len() + 1);
    affected.push(primary);
    affected.extend(removed_aliases);
    affected
}

fn insert_loaded_plugin(
    plugins: &mut HashMap<String, LoadedPlugin>,
    aliases: &mut HashMap<String, String>,
    record: LoadedPluginRecord,
    replace_existing_primary: bool,
    allow_alias_override: bool,
) -> Vec<String> {
    let mut affected = Vec::new();
    if plugins.contains_key(&record.primary_key) {
        if !replace_existing_primary {
            return affected;
        }
        affected.extend(remove_loaded_plugin(plugins, aliases, &record.primary_key));
    }

    let primary_key = record.primary_key.clone();
    let alias_keys = record.alias_keys.clone();
    plugins.insert(primary_key.clone(), record.loaded);
    affected.push(primary_key.clone());

    for alias in alias_keys {
        if plugins.contains_key(&alias) && alias != primary_key {
            continue;
        }
        if let Some(existing) = aliases.get(&alias)
            && existing != &primary_key
            && !allow_alias_override
        {
            continue;
        }
        aliases.insert(alias.clone(), primary_key.clone());
        affected.push(alias);
    }

    affected.sort();
    affected.dedup();
    affected
}

fn parse_builtin_descriptor(
    asset: crate::builtins::BuiltinPluginAsset,
) -> Result<PluginDescriptor, String> {
    serde_json::from_str(asset.descriptor_json)
        .map_err(|error| format!("built-in descriptor JSON is invalid: {error}"))
}

fn builtin_provider_types_from_assets(
    assets: &[crate::builtins::BuiltinPluginAsset],
    plugin_type_filter: impl Fn(&str) -> bool,
    apply_overrides: impl Fn(PluginDescriptor) -> PluginDescriptor,
) -> Vec<String> {
    assets
        .iter()
        .filter_map(|asset| parse_builtin_descriptor(*asset).ok())
        .map(apply_overrides)
        .filter(|descriptor| plugin_type_filter(descriptor.plugin_type()))
        .map(|descriptor| descriptor.provider_type().trim().to_ascii_lowercase())
        .collect()
}

fn builtin_indexer_provider_types() -> Vec<String> {
    static BUILTIN_INDEXER_PROVIDER_TYPES: LazyLock<Vec<String>> = LazyLock::new(|| {
        builtin_provider_types_from_assets(
            crate::builtins::INDEXER_BUILTINS,
            is_indexer_plugin_type,
            |descriptor| apply_indexer_provider_overrides(descriptor, PluginLoadSource::Builtin),
        )
    });

    BUILTIN_INDEXER_PROVIDER_TYPES.clone()
}

fn builtin_subtitle_provider_types() -> Vec<String> {
    static BUILTIN_SUBTITLE_PROVIDER_TYPES: LazyLock<Vec<String>> = LazyLock::new(|| {
        builtin_provider_types_from_assets(
            crate::builtins::SUBTITLE_BUILTINS,
            |plugin_type| plugin_type == "subtitle_provider",
            |descriptor| descriptor,
        )
    });

    BUILTIN_SUBTITLE_PROVIDER_TYPES.clone()
}

pub struct WasmIndexerPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
    aliases: HashMap<String, String>,
}

impl WasmIndexerPluginProvider {
    /// Create an empty provider with no plugins loaded.
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
            aliases: HashMap::new(),
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

    fn prepare_external_plugin_record(
        plugin: ExternalPluginWasm<'_>,
    ) -> Result<LoadedPluginRecord, String> {
        let (descriptor, wasm_bytes) = load_from_bytes(plugin.bytes)?;
        let descriptor = apply_indexer_provider_overrides(
            descriptor,
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        );
        if !validate_indexer_descriptor(
            &descriptor,
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("indexer descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            descriptor, wasm_bytes,
        )))
    }

    fn prepare_runtime_plugin_record(
        plugin: RuntimePluginLoad,
    ) -> Result<LoadedPluginRecord, String> {
        let descriptor = apply_indexer_provider_overrides(
            plugin.descriptor,
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        );
        if !validate_indexer_descriptor(
            &descriptor,
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("indexer descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            descriptor,
            plugin.wasm_bytes,
        )))
    }

    fn prepare_builtin_asset_record(
        asset: crate::builtins::BuiltinPluginAsset,
    ) -> Result<LoadedPluginRecord, String> {
        let descriptor = parse_builtin_descriptor(asset)?;
        let descriptor = apply_indexer_provider_overrides(descriptor, PluginLoadSource::Builtin);
        if !validate_indexer_descriptor(&descriptor, PluginLoadSource::Builtin) {
            return Err("built-in indexer descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_builtin(
            descriptor, asset,
        )))
    }

    fn with_external_plugin(mut self, plugin: ExternalPluginWasm<'_>) -> Self {
        match Self::prepare_external_plugin_record(plugin) {
            Ok(record) => {
                info!(
                    plugin = record.loaded.descriptor.name.as_str(),
                    version = record.loaded.descriptor.version.as_str(),
                    provider_type = record.primary_key.as_str(),
                    "registered external plugin"
                );
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load external plugin");
            }
        }
        self
    }

    fn with_runtime_plugin(mut self, plugin: RuntimePluginLoad) -> Self {
        match Self::prepare_runtime_plugin_record(plugin) {
            Ok(record) => {
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load runtime indexer plugin");
            }
        }
        self
    }

    fn restore_builtin_provider_type(
        &mut self,
        provider_type: &str,
    ) -> Result<Vec<String>, String> {
        let asset = builtin_indexer_asset_for_provider(provider_type).ok_or_else(|| {
            format!("no built-in indexer plugin is available for provider '{provider_type}'")
        })?;
        let record = Self::prepare_builtin_asset_record(asset)?;
        Ok(insert_loaded_plugin(
            &mut self.plugins,
            &mut self.aliases,
            record,
            false,
            false,
        ))
    }

    fn upsert_runtime_plugin_record(
        &mut self,
        plugin: RuntimePluginLoad,
    ) -> Result<Vec<String>, String> {
        let record = Self::prepare_runtime_plugin_record(plugin)?;
        Ok(insert_loaded_plugin(
            &mut self.plugins,
            &mut self.aliases,
            record,
            true,
            true,
        ))
    }

    fn remove_provider_type(&mut self, provider_type: &str) -> Vec<String> {
        remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type)
    }

    fn get_loaded(&self, provider_type: &str) -> Option<&LoadedPlugin> {
        resolve_loaded_plugin(&self.plugins, &self.aliases, provider_type)
    }

    /// Remove a provider_type (and its aliases) from the loaded set.
    /// Used to disable built-in plugins at runtime.
    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let _ = self.remove_provider_type(provider_type);
        self
    }

    pub fn with_builtin_asset(mut self, asset: crate::builtins::BuiltinPluginAsset) -> Self {
        match Self::prepare_builtin_asset_record(asset) {
            Ok(record) => {
                let _ = insert_loaded_plugin(
                    &mut self.plugins,
                    &mut self.aliases,
                    record,
                    false,
                    false,
                );
            }
            Err(error) => {
                warn!(error = %error, "failed to load built-in plugin");
            }
        }
        self
    }
}

fn apply_indexer_provider_overrides(
    mut descriptor: PluginDescriptor,
    load_source: PluginLoadSource,
) -> PluginDescriptor {
    let missing_connection_url = !descriptor
        .config_fields()
        .iter()
        .any(|field| field.role == Some(ConfigFieldRole::ConnectionUrl));
    if descriptor
        .provider_type()
        .eq_ignore_ascii_case("torrent_rss")
        && missing_connection_url
        && let Some(feed_url_field) = descriptor
            .config_fields_mut()
            .iter_mut()
            .find(|field| field.key.eq_ignore_ascii_case("feed_url"))
    {
        feed_url_field.role = Some(ConfigFieldRole::ConnectionUrl);
    }

    if matches!(load_source, PluginLoadSource::Builtin) {
        if missing_connection_url
            && matches!(
                descriptor.provider_type().to_ascii_lowercase().as_str(),
                "newznab" | "torznab" | "nzbgeek" | "dognzb"
            )
        {
            descriptor.config_fields_mut().insert(
                0,
                ConfigFieldDef {
                    key: "base_url".to_string(),
                    label: "Base URL".to_string(),
                    field_type: ConfigFieldType::String,
                    required: true,
                    default_value: None,
                    value_source: ConfigFieldValueSource::User,
                    role: Some(ConfigFieldRole::ConnectionUrl),
                    host_binding: None,
                    options: vec![],
                    help_text: Some(
                        "Base URL for the provider API endpoint, such as https://indexer.example/api"
                            .to_string(),
                    ),
                },
            );
        }

        if indexer_provider_requires_api_key(descriptor.provider_type())
            && !indexer_has_declared_api_key_field(&descriptor)
        {
            descriptor.config_fields_mut().push(ConfigFieldDef {
                key: "api_key".to_string(),
                label: "API Key".to_string(),
                field_type: ConfigFieldType::Password,
                required: true,
                default_value: None,
                value_source: ConfigFieldValueSource::User,
                role: None,
                host_binding: None,
                options: vec![],
                help_text: Some("API key used to authenticate search requests".to_string()),
            });
        }
    }

    if descriptor.provider_type().eq_ignore_ascii_case("nzbgeek") {
        descriptor.set_default_base_url(Some(NZBGEEK_DEFAULT_BASE_URL.to_string()));
        if let Some(base_url_field) = descriptor
            .config_fields_mut()
            .iter_mut()
            .find(|field| field.key == "base_url")
        {
            base_url_field.default_value = Some(NZBGEEK_DEFAULT_BASE_URL.to_string());
        }
    }
    if descriptor.provider_type().eq_ignore_ascii_case("dognzb") {
        descriptor.set_default_base_url(Some(DOGNZB_DEFAULT_BASE_URL.to_string()));
        descriptor
            .config_fields_mut()
            .retain(|field| field.key != "api_path" && field.key != "additional_params");
    }

    descriptor
}

fn builtin_indexer_asset_for_provider(
    provider_type: &str,
) -> Option<crate::builtins::BuiltinPluginAsset> {
    match provider_type.trim().to_ascii_lowercase().as_str() {
        "newznab" => Some(crate::builtins::NEWZNAB),
        "nzbgeek" => Some(crate::builtins::NZBGEEK),
        "torznab" => Some(crate::builtins::TORZNAB),
        _ => None,
    }
}

impl IndexerPluginProvider for WasmIndexerPluginProvider {
    fn available_provider_types(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        builtin_indexer_provider_types()
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.version.clone())
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.sdk_version.clone())
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| plugin_descriptor_sdk_constraint(&loaded.descriptor))
    }

    fn plugin_type_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
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
        self.get_loaded(provider_type)
            .map(|loaded| config_fields_to_domain(loaded.descriptor.config_fields()))
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        crate::builtins::builtin_description_for_provider(provider_type).map(str::to_string)
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .and_then(|loaded| default_indexer_connection_url(&loaded.descriptor))
    }

    fn rate_limit_seconds_for_provider(&self, provider_type: &str) -> Option<i64> {
        self.get_loaded(provider_type).and_then(|loaded| {
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
        self.get_loaded(provider_type)
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
        let loaded = self.get_loaded(&provider)?;

        let wasm_bytes = match loaded.materialize_wasm() {
            Ok(wasm_bytes) => wasm_bytes,
            Err(error) => {
                tracing::warn!(
                    indexer = config.name.as_str(),
                    provider = provider.as_str(),
                    error = %error,
                    "failed to materialize WASM indexer plugin bytes"
                );
                return None;
            }
        };

        match WasmIndexerClient::new(
            wasm_bytes,
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
    client_cache: IndexerClientCache,
}

impl DynamicPluginProvider {
    pub fn new(provider: WasmIndexerPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn invalidate_provider_keys(&self, provider_keys: &[String]) {
        if provider_keys.is_empty() {
            return;
        }
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.retain(|(provider_type, _, _), _| !provider_keys.contains(provider_type));
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
        let provider_key = config.provider_type.trim().to_ascii_lowercase();
        let cache_key = (
            provider_key.clone(),
            config.id.clone(),
            config.updated_at.to_rfc3339(),
        );

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

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicPluginProvider lock poisoned");
        guard.plugin_description_for_provider(provider_type)
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

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_indexer_plugin_provider_from_runtime_plugins(
            runtime_plugins,
            disabled_builtins,
        ));
        Ok(())
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicPluginProvider lock poisoned");
            guard.upsert_runtime_plugin_record(plugin)?
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicPluginProvider lock poisoned");
            guard.remove_provider_type(provider_type)
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }

    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicPluginProvider lock poisoned");
            guard.restore_builtin_provider_type(provider_type)?
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }
}

// ── Download client plugin provider ────────────────────────────────────

pub struct WasmDownloadClientPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
    aliases: HashMap<String, String>,
}

impl WasmDownloadClientPluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
            aliases: HashMap::new(),
        }
    }

    pub fn with_external_bytes(self, wasm_bytes: &[u8]) -> Self {
        self.with_external_plugin(ExternalPluginWasm {
            bytes: wasm_bytes,
            first_party: false,
        })
    }

    fn prepare_external_plugin_record(
        plugin: ExternalPluginWasm<'_>,
    ) -> Result<LoadedPluginRecord, String> {
        let (descriptor, wasm_bytes) = load_from_bytes(plugin.bytes)?;
        if !validate_descriptor_for_type(
            &descriptor,
            Some("download_client"),
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("download client descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            descriptor, wasm_bytes,
        )))
    }

    fn prepare_runtime_plugin_record(
        plugin: RuntimePluginLoad,
    ) -> Result<LoadedPluginRecord, String> {
        if !validate_descriptor_for_type(
            &plugin.descriptor,
            Some("download_client"),
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("download client descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            plugin.descriptor,
            plugin.wasm_bytes,
        )))
    }

    fn with_external_plugin(mut self, plugin: ExternalPluginWasm<'_>) -> Self {
        match Self::prepare_external_plugin_record(plugin) {
            Ok(record) => {
                info!(
                    plugin = record.loaded.descriptor.name.as_str(),
                    version = record.loaded.descriptor.version.as_str(),
                    provider_type = record.primary_key.as_str(),
                    "registered external download client plugin"
                );
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load external download client plugin");
            }
        }
        self
    }

    fn with_runtime_plugin(mut self, plugin: RuntimePluginLoad) -> Self {
        match Self::prepare_runtime_plugin_record(plugin) {
            Ok(record) => {
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load runtime download client plugin");
            }
        }
        self
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let _ = remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type);
        self
    }

    fn upsert_runtime_plugin_record(
        &mut self,
        plugin: RuntimePluginLoad,
    ) -> Result<Vec<String>, String> {
        let record = Self::prepare_runtime_plugin_record(plugin)?;
        Ok(insert_loaded_plugin(
            &mut self.plugins,
            &mut self.aliases,
            record,
            true,
            true,
        ))
    }

    fn remove_provider_type(&mut self, provider_type: &str) -> Vec<String> {
        remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type)
    }

    fn get_loaded(&self, provider_type: &str) -> Option<&LoadedPlugin> {
        resolve_loaded_plugin(&self.plugins, &self.aliases, provider_type)
    }

    fn create_download_client(
        loaded: &LoadedPlugin,
        config: &DownloadClientConfig,
    ) -> Option<Arc<dyn DownloadClient>> {
        let wasm_bytes = match loaded.materialize_wasm() {
            Ok(wasm_bytes) => wasm_bytes,
            Err(error) => {
                warn!(
                    client = config.name.as_str(),
                    provider_type = config.client_type.as_str(),
                    error = %error,
                    "failed to materialize WASM download client bytes"
                );
                return None;
            }
        };

        let mut manifest = Manifest::new([extism::Wasm::data(wasm_bytes)]);
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
        let loaded = self.get_loaded(&provider)?;
        Self::create_download_client(loaded, config)
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.version.clone())
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.sdk_version.clone())
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| plugin_descriptor_sdk_constraint(&loaded.descriptor))
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        self.get_loaded(provider_type)
            .map(|loaded| config_fields_to_domain(loaded.descriptor.config_fields()))
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        crate::builtins::builtin_description_for_provider(provider_type).map(str::to_string)
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .and_then(|loaded| loaded.descriptor.default_base_url().map(ToOwned::to_owned))
    }

    fn accepted_inputs_for_provider(&self, provider_type: &str) -> Vec<String> {
        self.get_loaded(provider_type)
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
    client_cache: DownloadClientCache,
}

impl DynamicDownloadClientPluginProvider {
    pub fn new(provider: WasmDownloadClientPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn invalidate_provider_keys(&self, provider_keys: &[String]) {
        if provider_keys.is_empty() {
            return;
        }
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.retain(|(provider_type, _, _), _| !provider_keys.contains(provider_type));
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
        let provider_key = config.client_type.trim().to_ascii_lowercase();
        let cache_key = (
            provider_key.clone(),
            config.id.clone(),
            config.updated_at.to_rfc3339(),
        );

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

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicDownloadClientPluginProvider lock poisoned");
        guard.plugin_description_for_provider(provider_type)
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

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_download_client_plugin_provider_from_runtime_plugins(
            runtime_plugins,
            disabled_builtins,
        ));
        Ok(())
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicDownloadClientPluginProvider lock poisoned");
            guard.upsert_runtime_plugin_record(plugin)?
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicDownloadClientPluginProvider lock poisoned");
            guard.remove_provider_type(provider_type)
        };
        self.invalidate_provider_keys(&affected);
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
    if descriptor.provider_type().eq_ignore_ascii_case("prowlarr") {
        warn!(
            plugin = descriptor.id.as_str(),
            provider_type = descriptor.provider_type(),
            "skipping plugin: prowlarr is reserved for the first-party provider"
        );
        return false;
    }
    validate_descriptor_for_type(descriptor, None, load_source)
        && is_indexer_plugin_type(descriptor.plugin_type())
        && validate_indexer_config_contract(descriptor)
}

fn validate_indexer_config_contract(descriptor: &PluginDescriptor) -> bool {
    let connection_url_count = descriptor
        .config_fields()
        .iter()
        .filter(|field| field.role == Some(ConfigFieldRole::ConnectionUrl))
        .count();

    let has_connection_url = match connection_url_count {
        1 => true,
        0 => {
            warn!(
                plugin = descriptor.id.as_str(),
                provider_type = descriptor.provider_type(),
                "indexer descriptor rejected: missing connection_url config field role"
            );
            false
        }
        _ => {
            warn!(
                plugin = descriptor.id.as_str(),
                provider_type = descriptor.provider_type(),
                "indexer descriptor rejected: multiple connection_url config field roles"
            );
            false
        }
    };
    if !has_connection_url {
        return false;
    }

    if indexer_provider_requires_api_key(descriptor.provider_type())
        && !indexer_has_declared_api_key_field(descriptor)
    {
        warn!(
            plugin = descriptor.id.as_str(),
            provider_type = descriptor.provider_type(),
            "indexer descriptor rejected: missing declared api_key config field"
        );
        return false;
    }

    true
}

fn indexer_provider_requires_api_key(provider_type: &str) -> bool {
    matches!(
        provider_type.trim().to_ascii_lowercase().as_str(),
        "newznab" | "torznab" | "nzbgeek" | "dognzb"
    )
}

fn indexer_has_declared_api_key_field(descriptor: &PluginDescriptor) -> bool {
    descriptor.config_fields().iter().any(|field| {
        field.key.eq_ignore_ascii_case("api_key")
            && field.field_type == ConfigFieldType::Password
            && field.value_source == ConfigFieldValueSource::User
    })
}

fn default_indexer_connection_url(descriptor: &PluginDescriptor) -> Option<String> {
    descriptor
        .config_fields()
        .iter()
        .find(|field| field.role == Some(ConfigFieldRole::ConnectionUrl))
        .and_then(|field| field.default_value.clone())
        .filter(|value| !value.trim().is_empty())
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

    for asset in crate::builtins::INDEXER_BUILTINS {
        provider = provider.with_builtin_asset(*asset);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

pub fn build_indexer_plugin_provider_from_runtime_plugins(
    runtime_plugins: &[RuntimePluginLoad],
    disabled_builtins: &[String],
) -> WasmIndexerPluginProvider {
    let mut provider = WasmIndexerPluginProvider::empty();

    for plugin in runtime_plugins.iter().cloned() {
        provider = provider.with_runtime_plugin(plugin);
    }

    for asset in crate::builtins::INDEXER_BUILTINS {
        provider = provider.with_builtin_asset(*asset);
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

pub fn build_download_client_plugin_provider_from_runtime_plugins(
    runtime_plugins: &[RuntimePluginLoad],
    disabled_builtins: &[String],
) -> WasmDownloadClientPluginProvider {
    let mut provider = WasmDownloadClientPluginProvider::empty();

    for plugin in runtime_plugins.iter().cloned() {
        provider = provider.with_runtime_plugin(plugin);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

// ── Subtitle provider plugin provider ────────────────────────────────

pub struct WasmSubtitlePluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
    aliases: HashMap<String, String>,
}

impl WasmSubtitlePluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
            aliases: HashMap::new(),
        }
    }

    pub fn with_external_bytes(self, wasm_bytes: &[u8]) -> Self {
        self.with_external_plugin(ExternalPluginWasm {
            bytes: wasm_bytes,
            first_party: false,
        })
    }

    fn prepare_external_plugin_record(
        plugin: ExternalPluginWasm<'_>,
    ) -> Result<LoadedPluginRecord, String> {
        let (descriptor, wasm_bytes) = load_from_bytes(plugin.bytes)?;
        if !validate_descriptor_for_type(
            &descriptor,
            Some("subtitle_provider"),
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("subtitle provider descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            descriptor, wasm_bytes,
        )))
    }

    fn prepare_runtime_plugin_record(
        plugin: RuntimePluginLoad,
    ) -> Result<LoadedPluginRecord, String> {
        if !validate_descriptor_for_type(
            &plugin.descriptor,
            Some("subtitle_provider"),
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("subtitle provider descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            plugin.descriptor,
            plugin.wasm_bytes,
        )))
    }

    fn prepare_builtin_asset_record(
        asset: crate::builtins::BuiltinPluginAsset,
    ) -> Result<LoadedPluginRecord, String> {
        let descriptor = parse_builtin_descriptor(asset)?;
        if !validate_descriptor_for_type(
            &descriptor,
            Some("subtitle_provider"),
            PluginLoadSource::Builtin,
        ) {
            return Err("built-in subtitle provider descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_builtin(
            descriptor, asset,
        )))
    }

    fn with_external_plugin(mut self, plugin: ExternalPluginWasm<'_>) -> Self {
        match Self::prepare_external_plugin_record(plugin) {
            Ok(record) => {
                info!(
                    plugin = record.loaded.descriptor.name.as_str(),
                    version = record.loaded.descriptor.version.as_str(),
                    provider_type = record.primary_key.as_str(),
                    "registered external subtitle provider plugin"
                );
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load external subtitle provider plugin");
            }
        }
        self
    }

    fn with_runtime_plugin(mut self, plugin: RuntimePluginLoad) -> Self {
        match Self::prepare_runtime_plugin_record(plugin) {
            Ok(record) => {
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load runtime subtitle provider plugin");
            }
        }
        self
    }

    pub fn with_builtin_asset(mut self, asset: crate::builtins::BuiltinPluginAsset) -> Self {
        match Self::prepare_builtin_asset_record(asset) {
            Ok(record) => {
                let _ = insert_loaded_plugin(
                    &mut self.plugins,
                    &mut self.aliases,
                    record,
                    false,
                    false,
                );
            }
            Err(error) => {
                warn!(error = %error, "failed to load built-in subtitle provider plugin");
            }
        }
        self
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let _ = remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type);
        self
    }

    fn restore_builtin_provider_type(
        &mut self,
        provider_type: &str,
    ) -> Result<Vec<String>, String> {
        let asset = builtin_subtitle_asset_for_provider(provider_type).ok_or_else(|| {
            format!("no built-in subtitle plugin is available for provider '{provider_type}'")
        })?;
        let record = Self::prepare_builtin_asset_record(asset)?;
        Ok(insert_loaded_plugin(
            &mut self.plugins,
            &mut self.aliases,
            record,
            false,
            false,
        ))
    }

    fn upsert_runtime_plugin_record(
        &mut self,
        plugin: RuntimePluginLoad,
    ) -> Result<Vec<String>, String> {
        let record = Self::prepare_runtime_plugin_record(plugin)?;
        Ok(insert_loaded_plugin(
            &mut self.plugins,
            &mut self.aliases,
            record,
            true,
            true,
        ))
    }

    fn remove_provider_type(&mut self, provider_type: &str) -> Vec<String> {
        remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type)
    }

    fn get_loaded(&self, provider_type: &str) -> Option<&LoadedPlugin> {
        resolve_loaded_plugin(&self.plugins, &self.aliases, provider_type)
    }

    fn audio_transcoder_client_from_loaded(
        loaded: &LoadedPlugin,
    ) -> Option<Arc<dyn AudioTranscoderClient>> {
        let wasm_bytes = loaded.materialize_wasm().ok()?;
        let client = WasmAudioTranscoderClient::new(wasm_bytes, loaded.descriptor.clone());
        Some(Arc::new(client))
    }
}

impl SubtitlePluginProvider for WasmSubtitlePluginProvider {
    fn client_for_config(
        &self,
        config: &SubtitleProviderConfig,
        host_bindings: &HashMap<PluginHostBindingId, String>,
    ) -> Option<Arc<dyn SubtitleProviderClient>> {
        let provider = config.provider_type.trim().to_ascii_lowercase();
        let loaded = self.get_loaded(&provider)?;
        let wasm_bytes = match loaded.materialize_wasm() {
            Ok(wasm_bytes) => wasm_bytes,
            Err(error) => {
                warn!(
                    subtitle_provider = config.name.as_str(),
                    provider_type = provider.as_str(),
                    error = %error,
                    "failed to materialize WASM subtitle provider bytes"
                );
                return None;
            }
        };
        match WasmSubtitleClient::new(
            wasm_bytes,
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

    fn audio_transcoder_client(&self) -> Option<Arc<dyn AudioTranscoderClient>> {
        self.plugins
            .values()
            .find(|loaded| loaded.descriptor.provider_type() == "enhanced-subtitle-sync")
            .and_then(Self::audio_transcoder_client_from_loaded)
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        builtin_subtitle_provider_types()
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.version.clone())
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.sdk_version.clone())
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| plugin_descriptor_sdk_constraint(&loaded.descriptor))
    }

    fn supports_catalog_search_for_provider(&self, provider_type: &str) -> bool {
        self.get_loaded(provider_type)
            .and_then(|loaded| loaded.descriptor.subtitle())
            .is_some_and(|subtitle| subtitle.capabilities.mode == SubtitleProviderMode::Catalog)
    }

    fn recommended_facets_for_provider(&self, provider_type: &str) -> Vec<String> {
        self.get_loaded(provider_type)
            .and_then(|loaded| loaded.descriptor.subtitle())
            .map(|subtitle| subtitle.capabilities.recommended_facets.clone())
            .unwrap_or_default()
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        self.get_loaded(provider_type)
            .map(|loaded| config_fields_to_domain(loaded.descriptor.config_fields()))
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        crate::builtins::builtin_description_for_provider(provider_type).map(str::to_string)
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

    fn invalidate_provider_keys(&self, provider_keys: &[String]) {
        if provider_keys.is_empty() {
            return;
        }
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.retain(|(provider_type, _, _, _, _), _| !provider_keys.contains(provider_type));
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
        let provider_key = config.provider_type.trim().to_ascii_lowercase();
        let cache_key = (
            provider_key.clone(),
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

    fn audio_transcoder_client(&self) -> Option<Arc<dyn AudioTranscoderClient>> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.audio_transcoder_client()
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

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicSubtitlePluginProvider lock poisoned");
        guard.plugin_description_for_provider(provider_type)
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

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_subtitle_plugin_provider_from_runtime_plugins(
            runtime_plugins,
            disabled_builtins,
        ));
        Ok(())
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicSubtitlePluginProvider lock poisoned");
            guard.upsert_runtime_plugin_record(plugin)?
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicSubtitlePluginProvider lock poisoned");
            guard.remove_provider_type(provider_type)
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }

    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicSubtitlePluginProvider lock poisoned");
            guard.restore_builtin_provider_type(provider_type)?
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }
}

fn builtin_subtitle_asset_for_provider(
    provider_type: &str,
) -> Option<crate::builtins::BuiltinPluginAsset> {
    let _ = provider_type;
    None
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

    for asset in crate::builtins::SUBTITLE_BUILTINS {
        provider = provider.with_builtin_asset(*asset);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

pub fn build_subtitle_plugin_provider_from_runtime_plugins(
    runtime_plugins: &[RuntimePluginLoad],
    disabled_builtins: &[String],
) -> WasmSubtitlePluginProvider {
    let mut provider = WasmSubtitlePluginProvider::empty();

    for plugin in runtime_plugins.iter().cloned() {
        provider = provider.with_runtime_plugin(plugin);
    }

    for asset in crate::builtins::SUBTITLE_BUILTINS {
        provider = provider.with_builtin_asset(*asset);
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
    let mut provider = WasmIndexerPluginProvider::empty();

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
                if provider.plugins.contains_key(&provider_type) {
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

                let record =
                    LoadedPluginRecord::new(LoadedPlugin::from_owned(descriptor, wasm_bytes));
                let _ = insert_loaded_plugin(
                    &mut provider.plugins,
                    &mut provider.aliases,
                    record,
                    true,
                    true,
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

    Ok(provider)
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
            serde_json::Value::String(value) => value.trim().to_string(),
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
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }

    url::Url::parse(trimmed)
        .ok()
        .and_then(|parsed| parsed.host_str().map(ToOwned::to_owned))
}

pub(crate) fn build_plugin(manifest: Manifest) -> Result<extism::Plugin, extism::Error> {
    build_plugin_with_socket_host(manifest, &SocketHost::disabled())
}

fn build_plugin_with_socket_host(
    manifest: Manifest,
    socket_host: &SocketHost,
) -> Result<extism::Plugin, extism::Error> {
    // Wasmtime's filesystem cache is not race-free when multiple identical
    // modules compile concurrently in the same process. Serialize the build
    // step so parallel provider/client loading does not emit cache rename
    // warnings for a benign same-artifact race.
    let _build_guard = WASMTIME_PLUGIN_BUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut functions = socket_host.functions();
    functions.extend(plugin_http_host::host_functions(&manifest));

    extism::PluginBuilder::new(manifest)
        .with_wasi(true)
        .with_http_response_headers(true)
        .with_functions(functions)
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

#[derive(Clone, Copy, Debug, Default)]
pub struct WasmPluginDescriptorLoader;

impl PluginDescriptorLoader for WasmPluginDescriptorLoader {
    fn load_descriptor_from_wasm_bytes(&self, wasm_bytes: &[u8]) -> AppResult<PluginDescriptor> {
        load_from_bytes(wasm_bytes)
            .map(|(descriptor, _)| descriptor)
            .map_err(AppError::Validation)
    }
}

// ── Notification plugin provider ───────────────────────────────────────

pub struct WasmNotificationPluginProvider {
    plugins: HashMap<String, LoadedPlugin>,
    aliases: HashMap<String, String>,
}

impl WasmNotificationPluginProvider {
    pub fn empty() -> Self {
        Self {
            plugins: HashMap::new(),
            aliases: HashMap::new(),
        }
    }

    pub fn with_external_bytes(self, wasm_bytes: &[u8]) -> Self {
        self.with_external_plugin(ExternalPluginWasm {
            bytes: wasm_bytes,
            first_party: false,
        })
    }

    fn prepare_external_plugin_record(
        plugin: ExternalPluginWasm<'_>,
    ) -> Result<LoadedPluginRecord, String> {
        let (descriptor, wasm_bytes) = load_from_bytes(plugin.bytes)?;
        if !validate_descriptor_for_type(
            &descriptor,
            Some("notification"),
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("notification descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            descriptor, wasm_bytes,
        )))
    }

    fn prepare_runtime_plugin_record(
        plugin: RuntimePluginLoad,
    ) -> Result<LoadedPluginRecord, String> {
        if !validate_descriptor_for_type(
            &plugin.descriptor,
            Some("notification"),
            PluginLoadSource::External {
                first_party: plugin.first_party,
            },
        ) {
            return Err("notification descriptor rejected".to_string());
        }
        Ok(LoadedPluginRecord::new(LoadedPlugin::from_owned(
            plugin.descriptor,
            plugin.wasm_bytes,
        )))
    }

    fn with_external_plugin(mut self, plugin: ExternalPluginWasm<'_>) -> Self {
        match Self::prepare_external_plugin_record(plugin) {
            Ok(record) => {
                info!(
                    plugin = record.loaded.descriptor.name.as_str(),
                    version = record.loaded.descriptor.version.as_str(),
                    provider_type = record.primary_key.as_str(),
                    "registered external notification plugin"
                );
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load external notification plugin");
            }
        }
        self
    }

    fn with_runtime_plugin(mut self, plugin: RuntimePluginLoad) -> Self {
        match Self::prepare_runtime_plugin_record(plugin) {
            Ok(record) => {
                let _ =
                    insert_loaded_plugin(&mut self.plugins, &mut self.aliases, record, true, true);
            }
            Err(error) => {
                warn!(error = %error, "failed to load runtime notification plugin");
            }
        }
        self
    }

    pub fn without_provider_type(mut self, provider_type: &str) -> Self {
        let _ = remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type);
        self
    }

    fn upsert_runtime_plugin_record(
        &mut self,
        plugin: RuntimePluginLoad,
    ) -> Result<Vec<String>, String> {
        let record = Self::prepare_runtime_plugin_record(plugin)?;
        Ok(insert_loaded_plugin(
            &mut self.plugins,
            &mut self.aliases,
            record,
            true,
            true,
        ))
    }

    fn remove_provider_type(&mut self, provider_type: &str) -> Vec<String> {
        remove_loaded_plugin(&mut self.plugins, &mut self.aliases, provider_type)
    }

    fn get_loaded(&self, provider_type: &str) -> Option<&LoadedPlugin> {
        resolve_loaded_plugin(&self.plugins, &self.aliases, provider_type)
    }

    fn create_notification_client(
        loaded: &LoadedPlugin,
        config: &NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>> {
        let wasm_bytes = match loaded.materialize_wasm() {
            Ok(wasm_bytes) => wasm_bytes,
            Err(error) => {
                warn!(
                    channel = config.name.as_str(),
                    error = %error,
                    "failed to materialize WASM notification plugin bytes"
                );
                return None;
            }
        };

        let mut manifest = Manifest::new([extism::Wasm::data(wasm_bytes)]);
        manifest = apply_allowed_hosts(
            manifest,
            &loaded.descriptor,
            None,
            Some(&config.config_json),
        );
        manifest = manifest.with_timeout(std::time::Duration::from_secs(30));
        let socket_host =
            SocketHost::from_descriptor(&loaded.descriptor, Some(&config.config_json));

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

        match build_plugin_with_socket_host(manifest, &socket_host) {
            Ok(plugin) => {
                let client = WasmNotificationClient::new(
                    plugin,
                    loaded.descriptor.clone(),
                    config.name.clone(),
                    Some(socket_host),
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
        let loaded = self.get_loaded(&provider)?;
        Self::create_notification_client(loaded, config)
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        vec![]
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.version.clone())
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.sdk_version.clone())
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| plugin_descriptor_sdk_constraint(&loaded.descriptor))
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        self.get_loaded(provider_type)
            .map(|loaded| config_fields_to_domain(loaded.descriptor.config_fields()))
            .unwrap_or_default()
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        self.get_loaded(provider_type)
            .map(|loaded| loaded.descriptor.name.clone())
    }

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        crate::builtins::builtin_description_for_provider(provider_type).map(str::to_string)
    }

    fn supported_events_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::NotificationEventType> {
        self.get_loaded(provider_type)
            .map(notification_supported_events_from_loaded)
            .unwrap_or_default()
    }

    fn supports_test_for_provider(&self, provider_type: &str) -> bool {
        self.get_loaded(provider_type)
            .map(notification_supports_test_from_loaded)
            .unwrap_or(false)
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
    client_cache: NotificationClientCache,
}

impl DynamicNotificationPluginProvider {
    pub fn new(provider: WasmNotificationPluginProvider) -> Self {
        Self {
            inner: std::sync::RwLock::new(provider),
            client_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn invalidate_provider_keys(&self, provider_keys: &[String]) {
        if provider_keys.is_empty() {
            return;
        }
        if let Ok(mut cache) = self.client_cache.lock() {
            cache.retain(|(provider_type, _, _), _| !provider_keys.contains(provider_type));
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
        let provider_key = config.channel_type.as_str().to_ascii_lowercase();
        let cache_key = (
            provider_key.clone(),
            config.id.clone(),
            config.updated_at.to_rfc3339(),
        );

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

    fn plugin_description_for_provider(&self, provider_type: &str) -> Option<String> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.plugin_description_for_provider(provider_type)
    }

    fn supported_events_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::NotificationEventType> {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.supported_events_for_provider(provider_type)
    }

    fn supports_test_for_provider(&self, provider_type: &str) -> bool {
        let guard = self
            .inner
            .read()
            .expect("DynamicNotificationPluginProvider lock poisoned");
        guard.supports_test_for_provider(provider_type)
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

    fn reload_runtime_plugins(
        &self,
        runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload(build_notification_plugin_provider_from_runtime_plugins(
            runtime_plugins,
            disabled_builtins,
        ));
        Ok(())
    }

    fn upsert_runtime_plugin(&self, plugin: RuntimePluginLoad) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicNotificationPluginProvider lock poisoned");
            guard.upsert_runtime_plugin_record(plugin)?
        };
        self.invalidate_provider_keys(&affected);
        Ok(())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        let affected = {
            let mut guard = self
                .inner
                .write()
                .expect("DynamicNotificationPluginProvider lock poisoned");
            guard.remove_provider_type(provider_type)
        };
        self.invalidate_provider_keys(&affected);
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

pub fn build_notification_plugin_provider_from_runtime_plugins(
    runtime_plugins: &[RuntimePluginLoad],
    disabled_builtins: &[String],
) -> WasmNotificationPluginProvider {
    let mut provider = WasmNotificationPluginProvider::empty();

    for plugin in runtime_plugins.iter().cloned() {
        provider = provider.with_runtime_plugin(plugin);
    }

    for provider_type in disabled_builtins {
        provider = provider.without_provider_type(provider_type);
    }

    provider
}

fn notification_supported_events_from_loaded(
    loaded: &LoadedPlugin,
) -> Vec<scryer_domain::NotificationEventType> {
    loaded
        .descriptor
        .notification()
        .map(|notification| {
            notification
                .capabilities
                .supported_events
                .iter()
                .filter_map(|event| scryer_domain::NotificationEventType::parse(event.as_str()))
                .collect()
        })
        .unwrap_or_default()
}

fn notification_supports_test_from_loaded(loaded: &LoadedPlugin) -> bool {
    loaded
        .descriptor
        .notification()
        .map(|notification| notification.capabilities.supports_test)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::{NEWZNAB, NZBGEEK, TORZNAB};
    use crate::types::{
        ConfigFieldDef, ConfigFieldType, ConfigFieldValueSource, DownloadClientCapabilities,
        DownloadClientDescriptor, IndexerDescriptor, IndexerSourceKind, NotificationCapabilities,
        NotificationDescriptor, PluginHostBindingId, SubtitleCapabilities, SubtitleDescriptor,
    };

    struct DummyIndexerClient;

    #[async_trait::async_trait]
    impl IndexerClient for DummyIndexerClient {
        async fn search(
            &self,
            _query: String,
            _ids: std::collections::HashMap<String, String>,
            _category: Option<String>,
            _facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<scryer_application::IndexerRoutingPlan>,
            _mode: scryer_application::SearchMode,
            _season: Option<u32>,
            _episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
        ) -> scryer_application::AppResult<scryer_application::IndexerSearchResponse> {
            unreachable!("dummy indexer client should not be called")
        }
    }

    struct DummyDownloadClient;

    #[async_trait::async_trait]
    impl DownloadClient for DummyDownloadClient {
        async fn submit_download(
            &self,
            _request: &scryer_application::DownloadClientAddRequest,
        ) -> scryer_application::AppResult<scryer_application::DownloadGrabResult> {
            unreachable!("dummy download client should not be called")
        }
    }

    struct DummyNotificationClient;

    #[async_trait::async_trait]
    impl NotificationClient for DummyNotificationClient {
        async fn send_notification(
            &self,
            _payload: &scryer_application::NotificationPayload,
        ) -> scryer_application::AppResult<()> {
            unreachable!("dummy notification client should not be called")
        }
    }

    fn indexer_config_fields() -> Vec<ConfigFieldDef> {
        vec![ConfigFieldDef {
            key: "base_url".to_string(),
            label: "Base URL".to_string(),
            field_type: ConfigFieldType::String,
            required: true,
            default_value: None,
            value_source: ConfigFieldValueSource::User,
            role: Some(ConfigFieldRole::ConnectionUrl),
            host_binding: None,
            options: vec![],
            help_text: None,
        }]
    }

    fn indexer_api_key_field() -> ConfigFieldDef {
        ConfigFieldDef {
            key: "api_key".to_string(),
            label: "API Key".to_string(),
            field_type: ConfigFieldType::Password,
            required: true,
            default_value: None,
            value_source: ConfigFieldValueSource::User,
            role: None,
            host_binding: None,
            options: vec![],
            help_text: None,
        }
    }

    fn descriptor(plugin_type: &str) -> PluginDescriptor {
        let provider = match plugin_type {
            "indexer" => ProviderDescriptor::Indexer(IndexerDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                source_kind: IndexerSourceKind::Generic,
                capabilities: Default::default(),
                scoring_policies: vec![],
                config_fields: indexer_config_fields(),
                allowed_hosts: vec![],
                rate_limit_seconds: None,
            }),
            "usenet_indexer" => ProviderDescriptor::Indexer(IndexerDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                source_kind: IndexerSourceKind::Usenet,
                capabilities: Default::default(),
                scoring_policies: vec![],
                config_fields: indexer_config_fields(),
                allowed_hosts: vec![],
                rate_limit_seconds: None,
            }),
            "torrent_indexer" => ProviderDescriptor::Indexer(IndexerDescriptor {
                provider_type: "test".to_string(),
                provider_aliases: vec![],
                source_kind: IndexerSourceKind::Torrent,
                capabilities: Default::default(),
                scoring_policies: vec![],
                config_fields: indexer_config_fields(),
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
            socket_permissions: vec![],
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

    fn set_provider_aliases(descriptor: &mut PluginDescriptor, aliases: Vec<String>) {
        match &mut descriptor.provider {
            ProviderDescriptor::Indexer(provider) => provider.provider_aliases = aliases,
            ProviderDescriptor::Notification(provider) => provider.provider_aliases = aliases,
            ProviderDescriptor::DownloadClient(provider) => provider.provider_aliases = aliases,
            ProviderDescriptor::Subtitle(provider) => provider.provider_aliases = aliases,
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

    fn runtime_plugin_load(
        plugin_type: &str,
        provider_type: &str,
        aliases: &[&str],
    ) -> RuntimePluginLoad {
        let mut descriptor = descriptor(plugin_type);
        descriptor.id = provider_type.to_string();
        descriptor.name = format!("{provider_type} plugin");
        descriptor.sdk_version = SDK_VERSION.to_string();
        descriptor.sdk_constraint = scryer_plugin_sdk::current_sdk_constraint();
        set_provider_type(&mut descriptor, provider_type);
        set_provider_aliases(
            &mut descriptor,
            aliases.iter().map(|alias| (*alias).to_string()).collect(),
        );
        if indexer_provider_requires_api_key(provider_type) {
            descriptor.config_fields_mut().push(indexer_api_key_field());
        }

        RuntimePluginLoad {
            descriptor,
            wasm_bytes: provider_type.as_bytes().to_vec(),
            first_party: true,
        }
    }

    #[test]
    fn embedded_builtin_descriptors_match_current_sdk_line() {
        for asset in [NEWZNAB, NZBGEEK, TORZNAB] {
            let descriptor: PluginDescriptor = serde_json::from_str(asset.descriptor_json)
                .expect("embedded builtin descriptor should parse");
            validate_plugin_descriptor_sdk_contract(&descriptor, SDK_VERSION)
                .expect("embedded builtin should match current SDK line");
        }
    }

    #[test]
    fn builtin_records_keep_embedded_assets_until_materialized() {
        let record = WasmIndexerPluginProvider::prepare_builtin_asset_record(NZBGEEK)
            .expect("builtin loads");
        assert!(record.loaded.stores_builtin_asset());

        let first = record
            .loaded
            .materialize_wasm()
            .expect("builtin should decode on demand");
        let second = record
            .loaded
            .materialize_wasm()
            .expect("builtin should decode on repeated access");

        assert!(!first.is_empty());
        assert_eq!(first, second);
    }

    #[test]
    fn builtin_providers_expose_embedded_plugins() {
        let indexers = build_indexer_plugin_provider(&[], &[]);
        for provider_type in ["newznab", "nzbgeek", "torznab"] {
            assert!(
                indexers.plugin_name_for_provider(provider_type).is_some(),
                "expected builtin indexer provider '{provider_type}' to be available"
            );
        }
        assert!(
            build_subtitle_plugin_provider(&[], &[])
                .builtin_provider_types()
                .is_empty()
        );
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
    fn auth_backed_indexers_require_declared_api_key_field() {
        for provider_type in ["newznab", "torznab", "nzbgeek", "dognzb"] {
            let mut descriptor = descriptor("usenet_indexer");
            set_provider_type(&mut descriptor, provider_type);
            assert!(
                !validate_indexer_descriptor(
                    &descriptor,
                    PluginLoadSource::External { first_party: false }
                ),
                "expected {provider_type} to require an api_key field"
            );

            let ProviderDescriptor::Indexer(indexer) = &mut descriptor.provider else {
                panic!("expected indexer descriptor");
            };
            indexer.config_fields.push(indexer_api_key_field());

            assert!(
                validate_indexer_descriptor(
                    &descriptor,
                    PluginLoadSource::External { first_party: false }
                ),
                "expected {provider_type} to validate once api_key is declared"
            );
        }
    }

    #[test]
    fn api_keyless_indexers_can_still_validate() {
        for provider_type in ["animetosho", "torrent_rss"] {
            let mut descriptor = descriptor("torrent_indexer");
            set_provider_type(&mut descriptor, provider_type);
            assert!(
                validate_indexer_descriptor(
                    &descriptor,
                    PluginLoadSource::External { first_party: false }
                ),
                "expected {provider_type} to validate without api_key"
            );
        }
    }

    #[test]
    fn torrent_rss_feed_url_is_backfilled_as_connection_url() {
        let mut descriptor = descriptor("torrent_indexer");
        set_provider_type(&mut descriptor, "torrent_rss");

        let ProviderDescriptor::Indexer(indexer) = &mut descriptor.provider else {
            panic!("expected indexer descriptor");
        };
        indexer.config_fields = vec![ConfigFieldDef {
            key: "feed_url".to_string(),
            label: "Feed URL".to_string(),
            field_type: ConfigFieldType::String,
            required: true,
            default_value: None,
            value_source: ConfigFieldValueSource::User,
            role: None,
            host_binding: None,
            options: vec![],
            help_text: None,
        }];

        let descriptor = apply_indexer_provider_overrides(
            descriptor,
            PluginLoadSource::External { first_party: false },
        );

        assert!(
            descriptor
                .config_fields()
                .iter()
                .find(|field| field.key == "feed_url")
                .is_some_and(|field| field.role == Some(ConfigFieldRole::ConnectionUrl))
        );
        assert!(validate_indexer_descriptor(
            &descriptor,
            PluginLoadSource::External { first_party: false }
        ));
    }

    #[test]
    fn auth_backed_builtin_indexers_backfill_base_url_and_api_key() {
        for provider_type in ["newznab", "torznab", "nzbgeek", "dognzb"] {
            let mut descriptor = descriptor("usenet_indexer");
            set_provider_type(&mut descriptor, provider_type);

            let ProviderDescriptor::Indexer(indexer) = &mut descriptor.provider else {
                panic!("expected indexer descriptor");
            };
            indexer.config_fields.clear();

            let descriptor =
                apply_indexer_provider_overrides(descriptor, PluginLoadSource::Builtin);

            assert!(
                descriptor
                    .config_fields()
                    .iter()
                    .any(|field| field.role == Some(ConfigFieldRole::ConnectionUrl))
            );
            assert!(descriptor.config_fields().iter().any(
                |field| field.key == "api_key" && field.field_type == ConfigFieldType::Password
            ));
            assert!(validate_indexer_descriptor(
                &descriptor,
                PluginLoadSource::Builtin
            ));
        }
    }

    #[test]
    fn auth_backed_external_indexers_without_declared_fields_are_rejected() {
        for provider_type in ["newznab", "torznab", "nzbgeek", "dognzb"] {
            let mut descriptor = descriptor("usenet_indexer");
            set_provider_type(&mut descriptor, provider_type);

            let ProviderDescriptor::Indexer(indexer) = &mut descriptor.provider else {
                panic!("expected indexer descriptor");
            };
            indexer.config_fields.clear();

            let descriptor = apply_indexer_provider_overrides(
                descriptor,
                PluginLoadSource::External { first_party: false },
            );

            assert!(!validate_indexer_descriptor(
                &descriptor,
                PluginLoadSource::External { first_party: false }
            ));
        }
    }

    #[test]
    fn animetosho_external_without_connection_field_is_rejected() {
        let mut descriptor = descriptor("torrent_indexer");
        set_provider_type(&mut descriptor, "animetosho");

        let ProviderDescriptor::Indexer(indexer) = &mut descriptor.provider else {
            panic!("expected indexer descriptor");
        };
        indexer.config_fields.clear();

        let descriptor = apply_indexer_provider_overrides(
            descriptor,
            PluginLoadSource::External { first_party: false },
        );

        assert!(!validate_indexer_descriptor(
            &descriptor,
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
            role: None,
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
            role: None,
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
            role: None,
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
            r#"{"username":" alice ","api_path":" /api ","use_ssl":false,"port":8080,"meta":{"tag":"series"}}"#,
        )
        .unwrap();

        assert_eq!(entries.get("username"), Some(&"alice".to_string()));
        assert_eq!(entries.get("api_path"), Some(&"/api".to_string()));
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

    #[test]
    fn indexer_runtime_mutation_invalidates_only_changed_provider_cache_entries() {
        let provider = DynamicPluginProvider::new(build_indexer_plugin_provider(&[], &[]));
        provider
            .upsert_runtime_plugin(runtime_plugin_load(
                "indexer",
                "animetosho",
                &["animetosho-alias"],
            ))
            .expect("upsert animetosho");
        provider
            .upsert_runtime_plugin(runtime_plugin_load("indexer", "newznab", &[]))
            .expect("upsert newznab");

        {
            let mut cache = provider.client_cache.lock().expect("indexer cache lock");
            cache.insert(
                (
                    "animetosho".to_string(),
                    "cfg-a".to_string(),
                    "1".to_string(),
                ),
                Arc::new(DummyIndexerClient),
            );
            cache.insert(
                (
                    "animetosho-alias".to_string(),
                    "cfg-b".to_string(),
                    "1".to_string(),
                ),
                Arc::new(DummyIndexerClient),
            );
            cache.insert(
                ("newznab".to_string(), "cfg-c".to_string(), "1".to_string()),
                Arc::new(DummyIndexerClient),
            );
        }

        provider
            .remove_runtime_plugin("animetosho")
            .expect("remove target provider");

        let cache = provider.client_cache.lock().expect("indexer cache lock");
        assert_eq!(cache.len(), 1);
        assert!(
            cache
                .keys()
                .all(|(provider_type, _, _)| provider_type == "newznab")
        );
        let providers = provider.available_provider_types();
        assert!(
            providers
                .iter()
                .any(|provider_type| provider_type == "newznab")
        );
        assert!(
            !providers
                .iter()
                .any(|provider_type| provider_type == "animetosho")
        );
    }

    #[test]
    fn download_runtime_mutation_invalidates_only_changed_provider_cache_entries() {
        let provider = DynamicDownloadClientPluginProvider::new(
            build_download_client_plugin_provider(&[], &[]),
        );
        provider
            .upsert_runtime_plugin(runtime_plugin_load(
                "download_client",
                "qbittorrent",
                &["qbt"],
            ))
            .expect("upsert qbittorrent");
        provider
            .upsert_runtime_plugin(runtime_plugin_load("download_client", "rtorrent", &[]))
            .expect("upsert rtorrent");

        {
            let mut cache = provider.client_cache.lock().expect("download cache lock");
            cache.insert(
                (
                    "qbittorrent".to_string(),
                    "cfg-a".to_string(),
                    "1".to_string(),
                ),
                Arc::new(DummyDownloadClient),
            );
            cache.insert(
                ("qbt".to_string(), "cfg-b".to_string(), "1".to_string()),
                Arc::new(DummyDownloadClient),
            );
            cache.insert(
                ("rtorrent".to_string(), "cfg-c".to_string(), "1".to_string()),
                Arc::new(DummyDownloadClient),
            );
        }

        provider
            .remove_runtime_plugin("qbittorrent")
            .expect("remove target provider");

        let cache = provider.client_cache.lock().expect("download cache lock");
        assert_eq!(cache.len(), 1);
        assert!(
            cache
                .keys()
                .all(|(provider_type, _, _)| provider_type == "rtorrent")
        );
        assert_eq!(
            provider.available_provider_types(),
            vec!["rtorrent".to_string()]
        );
    }

    #[test]
    fn notification_runtime_mutation_invalidates_only_changed_provider_cache_entries() {
        let provider =
            DynamicNotificationPluginProvider::new(build_notification_plugin_provider(&[], &[]));
        provider
            .upsert_runtime_plugin(runtime_plugin_load(
                "notification",
                "email",
                &["smtp-email"],
            ))
            .expect("upsert email");
        provider
            .upsert_runtime_plugin(runtime_plugin_load("notification", "webhook", &[]))
            .expect("upsert webhook");

        {
            let mut cache = provider
                .client_cache
                .lock()
                .expect("notification cache lock");
            cache.insert(
                (
                    "email".to_string(),
                    "channel-a".to_string(),
                    "1".to_string(),
                ),
                Arc::new(DummyNotificationClient),
            );
            cache.insert(
                (
                    "smtp-email".to_string(),
                    "channel-b".to_string(),
                    "1".to_string(),
                ),
                Arc::new(DummyNotificationClient),
            );
            cache.insert(
                (
                    "webhook".to_string(),
                    "channel-c".to_string(),
                    "1".to_string(),
                ),
                Arc::new(DummyNotificationClient),
            );
        }

        provider
            .remove_runtime_plugin("email")
            .expect("remove target provider");

        let cache = provider
            .client_cache
            .lock()
            .expect("notification cache lock");
        assert_eq!(cache.len(), 1);
        assert!(
            cache
                .keys()
                .all(|(provider_type, _, _)| provider_type == "webhook")
        );
        assert_eq!(
            provider.available_provider_types(),
            vec!["webhook".to_string()]
        );
    }

    #[test]
    fn subtitle_builtin_restore_rejects_removed_builtin() {
        let provider = DynamicSubtitlePluginProvider::new(build_subtitle_plugin_provider(&[], &[]));
        provider
            .upsert_runtime_plugin(runtime_plugin_load(
                "subtitle_provider",
                "opensubtitles",
                &[],
            ))
            .expect("upsert opensubtitles");

        let providers = provider.available_provider_types();
        assert!(
            providers
                .iter()
                .any(|provider_type| provider_type == "opensubtitles")
        );
        assert!(
            !providers
                .iter()
                .any(|provider_type| provider_type == "jimaku")
        );

        assert!(provider.restore_builtin_plugin("jimaku").is_err());
        let providers = provider.available_provider_types();
        assert!(
            providers
                .iter()
                .any(|provider_type| provider_type == "opensubtitles")
        );
        assert!(
            providers
                .iter()
                .all(|provider_type| provider_type != "jimaku")
        );
    }
}
