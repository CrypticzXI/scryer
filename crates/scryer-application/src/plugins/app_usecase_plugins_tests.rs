use super::*;
use async_trait::async_trait;
use scryer_domain::{
    NotificationChannelConfig, PersistedPluginWasmPayload, PluginHostBindingId, PluginSupportTier,
    PluginWasmEncoding,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc as StdArc, Mutex as StdMutex};
use tokio::sync::Mutex;

// ── Mock: PluginInstallationRepository ───────────────────────────────────────

struct MockPluginInstallationRepo {
    installations: Arc<Mutex<Vec<PluginInstallation>>>,
    payloads: Arc<Mutex<HashMap<String, PersistedPluginWasmPayload>>>,
    catalog_sources: Arc<Mutex<Vec<scryer_domain::PluginCatalogSource>>>,
    seeded: Arc<Mutex<Vec<SeededPluginRecord>>>,
    get_enabled_payload_calls: Arc<AtomicUsize>,
    get_single_payload_calls: Arc<AtomicUsize>,
}

type SeededPluginRecord = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
);

#[derive(Clone, Debug, Deserialize)]
struct CatalogFixtureManifest {
    #[serde(default)]
    plugins: Vec<CatalogFixtureEntry>,
    #[serde(default)]
    rule_packs: Vec<RulePackCatalogEntry>,
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogFixtureEntry {
    id: String,
    name: String,
    description: String,
    plugin_type: String,
    provider_type: String,
    #[serde(default = "catalog_fixture_default_official")]
    official: bool,
    #[serde(default)]
    releases: Vec<CatalogFixtureRelease>,
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogFixtureRelease {
    version: String,
    #[serde(default = "catalog_fixture_default_sdk_constraint")]
    sdk_constraint: String,
    #[serde(default)]
    builtin: bool,
    #[serde(default)]
    wasm_url: Option<String>,
}

fn catalog_fixture_default_official() -> bool {
    true
}

fn catalog_fixture_default_sdk_constraint() -> String {
    scryer_plugin_sdk::current_sdk_constraint()
}

impl MockPluginInstallationRepo {
    fn new() -> Self {
        Self {
            installations: Arc::new(Mutex::new(vec![])),
            payloads: Arc::new(Mutex::new(HashMap::new())),
            catalog_sources: Arc::new(Mutex::new(vec![])),
            seeded: Arc::new(Mutex::new(vec![])),
            get_enabled_payload_calls: Arc::new(AtomicUsize::new(0)),
            get_single_payload_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    async fn store_catalog_fixture_json(&self, json: &str) -> AppResult<()> {
        let fixture: CatalogFixtureManifest = serde_json::from_str(json).map_err(|error| {
            AppError::Validation(format!("invalid catalog fixture JSON: {error}"))
        })?;
        self.store_catalog_fixture(&fixture).await
    }

    async fn store_catalog_fixture(&self, fixture: &CatalogFixtureManifest) -> AppResult<()> {
        let mut sources = self.catalog_sources.lock().await;
        sources.retain(|source| {
            source.source_key != CENTRAL_CATALOG_SOURCE_KEY
                && !source.source_key.starts_with("child:")
        });

        if fixture.plugins.is_empty() && fixture.rule_packs.is_empty() {
            return Ok(());
        }

        let mut central_plugins = Vec::new();
        let now = Utc::now();

        for plugin in &fixture.plugins {
            let source_repo = fixture_source_repo(&plugin.id);
            let github_repo = GitHubRepo::parse(&source_repo)?;
            central_plugins.push(CentralCatalogEntry {
                id: plugin.id.clone(),
                name: plugin.name.clone(),
                description: plugin.description.clone(),
                plugin_type: plugin.plugin_type.clone(),
                provider_type: plugin.provider_type.clone(),
                publisher: "scryer".to_string(),
                support_tier: if plugin.official {
                    PluginSupportTier::Official
                } else {
                    PluginSupportTier::Unverified
                },
                docs_url: format!("https://example.com/{}/docs", plugin.id),
                source_repo: source_repo.clone(),
                child_catalog_url: fixture_child_catalog_url(
                    &github_repo,
                    plugin
                        .releases
                        .first()
                        .map(|release| release.version.as_str()),
                ),
                required_signer: RequiredSigner {
                    github_repository: github_repo.slug(),
                    github_workflow: None,
                },
            });

            let releases = plugin
                .releases
                .iter()
                .filter(|release| !release.builtin)
                .filter_map(|release| {
                    release.wasm_url.as_ref()?;
                    Some(ChildCatalogRelease {
                        version: release.version.clone(),
                        sdk_constraint: release.sdk_constraint.clone(),
                        artifact_manifest_url: format!(
                            "{}v{}/plugin.manifest.json",
                            github_repo.release_asset_prefix(),
                            release.version
                        ),
                    })
                })
                .collect::<Vec<_>>();

            let child = ChildCatalog {
                schema_version: "scryer.plugin.child_catalog.v2".to_string(),
                id: plugin.id.clone(),
                name: plugin.name.clone(),
                description: plugin.description.clone(),
                plugin_type: plugin.plugin_type.clone(),
                provider_type: plugin.provider_type.clone(),
                publisher: "scryer".to_string(),
                support_tier: if plugin.official {
                    PluginSupportTier::Official
                } else {
                    PluginSupportTier::Unverified
                },
                docs_url: format!("https://example.com/{}/docs", plugin.id),
                source_repo,
                releases,
            };

            sources.push(scryer_domain::PluginCatalogSource {
                source_key: child_catalog_source_key(&plugin.id),
                source_kind: "child".to_string(),
                source_url: fixture_child_catalog_url(
                    &github_repo,
                    plugin
                        .releases
                        .first()
                        .map(|release| release.version.as_str()),
                ),
                github_repo: Some(github_repo.slug()),
                support_tier: if plugin.official {
                    PluginSupportTier::Official
                } else {
                    PluginSupportTier::Unverified
                },
                catalog_json: Some(serde_json::to_string(&child).expect("serialize child catalog")),
                last_success_at: Some(now),
                last_error: None,
                updated_at: now,
            });
        }

        let central = CentralCatalog {
            schema_version: "scryer.plugin.catalog.v2".to_string(),
            plugins: central_plugins,
            rule_packs: fixture.rule_packs.clone(),
        };

        sources.push(scryer_domain::PluginCatalogSource {
            source_key: CENTRAL_CATALOG_SOURCE_KEY.to_string(),
            source_kind: "central".to_string(),
            source_url: plugin_catalog_url(),
            github_repo: Some(CENTRAL_CATALOG_REPO.to_string()),
            support_tier: PluginSupportTier::Official,
            catalog_json: Some(serde_json::to_string(&central).expect("serialize central catalog")),
            last_success_at: Some(now),
            last_error: None,
            updated_at: now,
        });

        Ok(())
    }

    async fn store_raw_catalog_source(
        &self,
        source_key: &str,
        source_kind: &str,
        catalog_json: Option<String>,
    ) {
        let source = scryer_domain::PluginCatalogSource {
            source_key: source_key.to_string(),
            source_kind: source_kind.to_string(),
            source_url: "https://example.com/catalog.json".to_string(),
            github_repo: None,
            support_tier: PluginSupportTier::Official,
            catalog_json,
            last_success_at: Some(Utc::now()),
            last_error: None,
            updated_at: Utc::now(),
        };
        let mut sources = self.catalog_sources.lock().await;
        if let Some(existing) = sources
            .iter_mut()
            .find(|existing| existing.source_key == source.source_key)
        {
            *existing = source;
        } else {
            sources.push(source);
        }
    }
}

#[async_trait]
impl PluginInstallationRepository for MockPluginInstallationRepo {
    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>> {
        Ok(self.installations.lock().await.clone())
    }

    async fn get_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PluginInstallation>> {
        let list = self.installations.lock().await;
        Ok(list.iter().find(|i| i.plugin_id == plugin_id).cloned())
    }

    async fn create_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        let mut list = self.installations.lock().await;
        list.push(installation.clone());
        if let Some(wasm_bytes) = wasm_bytes {
            self.payloads.lock().await.insert(
                installation.plugin_id.clone(),
                PersistedPluginWasmPayload {
                    encoding: installation.wasm_encoding,
                    bytes: wasm_bytes.to_vec(),
                },
            );
        }
        Ok(installation.clone())
    }

    async fn update_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        let mut list = self.installations.lock().await;
        if let Some(existing) = list
            .iter_mut()
            .find(|i| i.plugin_id == installation.plugin_id)
        {
            *existing = installation.clone();
        }
        let mut payloads = self.payloads.lock().await;
        match wasm_bytes {
            Some(wasm_bytes) => {
                payloads.insert(
                    installation.plugin_id.clone(),
                    PersistedPluginWasmPayload {
                        encoding: installation.wasm_encoding,
                        bytes: wasm_bytes.to_vec(),
                    },
                );
            }
            None if installation.source_kind == PluginSourceKind::Bundled => {
                payloads.remove(&installation.plugin_id);
            }
            None => {}
        }
        Ok(installation.clone())
    }

    async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()> {
        let mut list = self.installations.lock().await;
        list.retain(|i| i.plugin_id != plugin_id);
        self.payloads.lock().await.remove(plugin_id);
        Ok(())
    }

    async fn get_enabled_plugin_wasm_bytes(
        &self,
    ) -> AppResult<Vec<(PluginInstallation, Option<PersistedPluginWasmPayload>)>> {
        self.get_enabled_payload_calls
            .fetch_add(1, Ordering::Relaxed);
        let list = self.installations.lock().await;
        let payloads = self.payloads.lock().await;
        Ok(list
            .iter()
            .filter(|i| i.is_enabled)
            .map(|i| (i.clone(), payloads.get(&i.plugin_id).cloned()))
            .collect())
    }

    async fn get_plugin_installation_wasm_payload(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PersistedPluginWasmPayload>> {
        self.get_single_payload_calls
            .fetch_add(1, Ordering::Relaxed);
        Ok(self.payloads.lock().await.get(plugin_id).cloned())
    }

    async fn seed_builtin(
        &self,
        plugin_id: &str,
        name: &str,
        description: &str,
        version: &str,
        sdk_version: &str,
        sdk_constraint: &str,
        plugin_type: &str,
        provider_type: &str,
    ) -> AppResult<()> {
        self.seeded.lock().await.push((
            plugin_id.to_string(),
            name.to_string(),
            description.to_string(),
            version.to_string(),
            sdk_version.to_string(),
            sdk_constraint.to_string(),
            plugin_type.to_string(),
            provider_type.to_string(),
        ));
        let mut installations = self.installations.lock().await;
        if installations
            .iter()
            .any(|existing| existing.plugin_id == plugin_id)
        {
            return Ok(());
        }

        let now = Utc::now();
        installations.push(PluginInstallation {
            id: plugin_id.to_string(),
            plugin_id: plugin_id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            version: version.to_string(),
            sdk_version: sdk_version.to_string(),
            sdk_constraint: sdk_constraint.to_string(),
            scryer_constraint: None,
            plugin_type: plugin_type.to_string(),
            provider_type: provider_type.to_string(),
            source_kind: PluginSourceKind::Bundled,
            is_enabled: true,
            is_builtin: true,
            wasm_encoding: PluginWasmEncoding::Identity,
            wasm_digest_algo: None,
            source_url: None,
            support_tier: PluginSupportTier::Official,
            publisher: Some("scryer".to_string()),
            docs_url: None,
            source_repo: None,
            manifest_url: None,
            wasm_digest: None,
            artifact_digest: None,
            descriptor_json: None,
            installed_at: now,
            updated_at: now,
        });
        Ok(())
    }

    async fn upsert_plugin_catalog_source(
        &self,
        source: &scryer_domain::PluginCatalogSource,
    ) -> AppResult<()> {
        let mut sources = self.catalog_sources.lock().await;
        if let Some(existing) = sources
            .iter_mut()
            .find(|existing| existing.source_key == source.source_key)
        {
            *existing = source.clone();
        } else {
            sources.push(source.clone());
        }
        Ok(())
    }

    async fn list_plugin_catalog_sources(
        &self,
    ) -> AppResult<Vec<scryer_domain::PluginCatalogSource>> {
        Ok(self.catalog_sources.lock().await.clone())
    }

    async fn get_plugin_catalog_source(
        &self,
        source_key: &str,
    ) -> AppResult<Option<scryer_domain::PluginCatalogSource>> {
        Ok(self
            .catalog_sources
            .lock()
            .await
            .iter()
            .find(|source| source.source_key == source_key)
            .cloned())
    }

    async fn upsert_plugin_catalog_status(
        &self,
        _status: &scryer_domain::PluginCatalogStatusRecord,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn get_plugin_catalog_status(
        &self,
        _status_key: &str,
    ) -> AppResult<Option<scryer_domain::PluginCatalogStatusRecord>> {
        Ok(None)
    }
}

// ── Mock: IndexerConfigRepository ────────────────────────────────────────────

struct MockIndexerConfigRepo {
    store: Arc<Mutex<Vec<IndexerConfig>>>,
}

impl MockIndexerConfigRepo {
    fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(vec![])),
        }
    }
}

#[async_trait]
impl IndexerConfigRepository for MockIndexerConfigRepo {
    async fn list(&self, provider_filter: Option<String>) -> AppResult<Vec<IndexerConfig>> {
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .filter(|e| {
                provider_filter
                    .as_ref()
                    .is_none_or(|pf| pf == &e.provider_type)
            })
            .cloned()
            .collect())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
        let entries = self.store.lock().await;
        Ok(entries.iter().find(|e| e.id == id).cloned())
    }

    async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
        self.store.lock().await.push(config.clone());
        Ok(config)
    }

    async fn touch_last_error(&self, _provider_type: &str) -> AppResult<()> {
        Ok(())
    }

    async fn update(&self, _update: crate::IndexerConfigUpdate) -> AppResult<IndexerConfig> {
        Err(AppError::Repository("not implemented".into()))
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.store.lock().await.retain(|e| e.id != id);
        Ok(())
    }
}

// ── Mock: IndexerPluginProvider ──────────────────────────────────────────────

struct MockPluginProvider {
    types: Vec<String>,
    builtin_types: Vec<String>,
    disabled_builtin_types: StdArc<StdMutex<Vec<String>>>,
    default_urls: HashMap<String, String>,
    plugin_names: HashMap<String, String>,
    plugin_versions: HashMap<String, String>,
    plugin_sdk_versions: HashMap<String, String>,
    plugin_sdk_constraints: HashMap<String, String>,
    plugin_types: HashMap<String, String>,
    removed_provider_types: StdArc<StdMutex<Vec<String>>>,
    reload_count: AtomicUsize,
    upsert_count: AtomicUsize,
    remove_count: AtomicUsize,
    restore_count: AtomicUsize,
}

impl MockPluginProvider {
    fn new() -> Self {
        Self {
            types: vec![],
            builtin_types: vec![],
            disabled_builtin_types: StdArc::new(StdMutex::new(vec![])),
            default_urls: HashMap::new(),
            plugin_names: HashMap::new(),
            plugin_versions: HashMap::new(),
            plugin_sdk_versions: HashMap::new(),
            plugin_sdk_constraints: HashMap::new(),
            plugin_types: HashMap::new(),
            removed_provider_types: StdArc::new(StdMutex::new(vec![])),
            reload_count: AtomicUsize::new(0),
            upsert_count: AtomicUsize::new(0),
            remove_count: AtomicUsize::new(0),
            restore_count: AtomicUsize::new(0),
        }
    }

    fn with_provider(mut self, pt: &str, name: &str, default_url: Option<&str>) -> Self {
        self.types.push(pt.to_string());
        self.plugin_names.insert(pt.to_string(), name.to_string());
        self.plugin_versions
            .insert(pt.to_string(), "0.1.0".to_string());
        self.plugin_sdk_versions
            .insert(pt.to_string(), scryer_plugin_sdk::SDK_VERSION.to_string());
        self.plugin_sdk_constraints
            .insert(pt.to_string(), scryer_plugin_sdk::current_sdk_constraint());
        self.plugin_types
            .insert(pt.to_string(), "indexer".to_string());
        if let Some(url) = default_url {
            self.default_urls.insert(pt.to_string(), url.to_string());
        }
        self
    }

    fn with_builtin_provider(mut self, pt: &str, name: &str, default_url: Option<&str>) -> Self {
        self = self.with_provider(pt, name, default_url);
        self.builtin_types.push(pt.to_string());
        self
    }

    fn runtime_provider_types(&self) -> Vec<String> {
        let disabled = self
            .disabled_builtin_types
            .lock()
            .expect("disabled builtin types lock");
        self.types
            .iter()
            .filter(|provider_type| {
                !self
                    .builtin_types
                    .iter()
                    .any(|builtin| builtin == *provider_type)
                    || !disabled.iter().any(|value| value == *provider_type)
            })
            .cloned()
            .collect()
    }
}

impl IndexerPluginProvider for MockPluginProvider {
    fn client_for_provider(&self, _config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        None
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.runtime_provider_types()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        let disabled = self
            .disabled_builtin_types
            .lock()
            .expect("disabled builtin types lock");
        self.builtin_types
            .iter()
            .filter(|provider_type| !disabled.iter().any(|value| value == *provider_type))
            .cloned()
            .collect()
    }

    fn plugin_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.plugin_versions.get(provider_type).cloned()
    }

    fn plugin_sdk_version_for_provider(&self, provider_type: &str) -> Option<String> {
        self.plugin_sdk_versions.get(provider_type).cloned()
    }

    fn plugin_sdk_constraint_for_provider(&self, provider_type: &str) -> Option<String> {
        self.plugin_sdk_constraints.get(provider_type).cloned()
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        if let Some(default_url) = self.default_urls.get(provider_type) {
            return vec![scryer_domain::ConfigFieldDef {
                key: "base_url".to_string(),
                label: "Base URL".to_string(),
                field_type: scryer_domain::ConfigFieldType::String,
                required: true,
                default_value: Some(default_url.clone()),
                value_source: scryer_domain::ConfigFieldValueSource::User,
                role: Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                host_binding: None,
                options: Vec::new(),
                help_text: None,
            }];
        }

        Vec::new()
    }

    fn plugin_type_for_provider(&self, provider_type: &str) -> Option<String> {
        self.plugin_types.get(provider_type).cloned()
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        vec![]
    }

    fn reload_plugins(
        &self,
        _external_wasm_bytes: &[ExternalPluginWasm<'_>],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        *self
            .disabled_builtin_types
            .lock()
            .expect("disabled builtin types lock") = disabled_builtins.to_vec();
        self.reload_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn reload_runtime_plugins(
        &self,
        _runtime_plugins: &[RuntimePluginLoad],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        *self
            .disabled_builtin_types
            .lock()
            .expect("disabled builtin types lock") = disabled_builtins.to_vec();
        self.reload_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn upsert_runtime_plugin(&self, _plugin: RuntimePluginLoad) -> Result<(), String> {
        self.upsert_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        if self
            .builtin_types
            .iter()
            .any(|builtin| builtin == provider_type)
        {
            let mut disabled = self
                .disabled_builtin_types
                .lock()
                .expect("disabled builtin types lock");
            if !disabled.iter().any(|value| value == provider_type) {
                disabled.push(provider_type.to_string());
            }
        }
        self.removed_provider_types
            .lock()
            .expect("removed provider types lock")
            .push(provider_type.to_string());
        self.remove_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let mut disabled = self
            .disabled_builtin_types
            .lock()
            .expect("disabled builtin types lock");
        disabled.retain(|value| value != provider_type);
        self.restore_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        self.plugin_names.get(provider_type).cloned()
    }

    fn default_base_url_for_provider(&self, provider_type: &str) -> Option<String> {
        self.default_urls.get(provider_type).cloned()
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn admin() -> User {
    let mut user = User::new_admin("admin");
    user.authorization = scryer_domain::UserAuthorization {
        app: scryer_domain::AppPermissionMask::from_permissions([
            scryer_domain::AppPermission::ManageUsers,
            scryer_domain::AppPermission::ManagePermissions,
            scryer_domain::AppPermission::ManageSystemSettings,
            scryer_domain::AppPermission::ManageCatalogSettings,
        ]),
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::ManageTitles,
            scryer_domain::LibraryPermission::ResolveImports,
            scryer_domain::LibraryPermission::ManageLibrary,
            scryer_domain::LibraryPermission::Request,
            scryer_domain::LibraryPermission::AutoApproveRequests,
        ]),
        loaded: true,
        ..Default::default()
    };
    user
}

fn viewer() -> User {
    User {
        id: scryer_domain::Id::new().0,
        username: "viewer".to_string(),
        password_hash: None,
        authorization: scryer_domain::UserAuthorization {
            default_library: scryer_domain::LibraryPermissionMask::from_permissions([
                scryer_domain::LibraryPermission::View,
            ]),
            loaded: true,
            ..Default::default()
        },
    }
}

fn config_admin() -> User {
    User {
        id: scryer_domain::Id::new().0,
        username: "config-admin".to_string(),
        password_hash: None,
        authorization: scryer_domain::UserAuthorization {
            app: scryer_domain::AppPermissionMask::from_permissions([
                scryer_domain::AppPermission::ManageSystemSettings,
                scryer_domain::AppPermission::ManageCatalogSettings,
            ]),
            loaded: true,
            ..Default::default()
        },
    }
}

fn make_installation(
    plugin_id: &str,
    version: &str,
    builtin: bool,
    enabled: bool,
) -> PluginInstallation {
    make_installation_with_type(plugin_id, version, "indexer", plugin_id, builtin, enabled)
}

fn make_installation_with_type(
    plugin_id: &str,
    version: &str,
    plugin_type: &str,
    provider_type: &str,
    builtin: bool,
    enabled: bool,
) -> PluginInstallation {
    let now = Utc::now();
    let descriptor_json = if builtin {
        None
    } else {
        let mut runtime_plugin = make_runtime_plugin_load(plugin_id, plugin_type, provider_type);
        runtime_plugin.descriptor.version = version.to_string();
        Some(
            serde_json::to_string(&runtime_plugin.descriptor)
                .expect("serialize test runtime plugin descriptor"),
        )
    };
    PluginInstallation {
        id: scryer_domain::Id::new().0,
        plugin_id: plugin_id.to_string(),
        name: format!("{plugin_id} Plugin"),
        description: format!("Description for {plugin_id}"),
        version: version.to_string(),
        sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
        sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
        scryer_constraint: None,
        plugin_type: plugin_type.to_string(),
        provider_type: provider_type.to_string(),
        source_kind: if builtin {
            scryer_domain::PluginSourceKind::Bundled
        } else {
            scryer_domain::PluginSourceKind::Downloaded
        },
        is_enabled: enabled,
        is_builtin: builtin,
        wasm_encoding: PluginWasmEncoding::Identity,
        wasm_digest_algo: None,
        source_url: None,
        support_tier: scryer_domain::PluginSupportTier::Official,
        publisher: None,
        docs_url: None,
        source_repo: None,
        manifest_url: None,
        wasm_digest: None,
        artifact_digest: None,
        descriptor_json,
        installed_at: now,
        updated_at: now,
    }
}

fn make_runtime_plugin_load(
    plugin_id: &str,
    plugin_type: &str,
    provider_type: &str,
) -> RuntimePluginLoad {
    fn indexer_config_fields() -> Vec<scryer_plugin_sdk::ConfigFieldDef> {
        vec![scryer_plugin_sdk::ConfigFieldDef {
            key: "base_url".to_string(),
            label: "Base URL".to_string(),
            field_type: scryer_plugin_sdk::ConfigFieldType::String,
            required: true,
            default_value: None,
            value_source: scryer_plugin_sdk::ConfigFieldValueSource::User,
            role: Some(scryer_plugin_sdk::ConfigFieldRole::ConnectionUrl),
            host_binding: None,
            options: vec![],
            help_text: None,
        }]
    }

    let provider = match plugin_type {
        "indexer" => {
            scryer_plugin_sdk::ProviderDescriptor::Indexer(scryer_plugin_sdk::IndexerDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: vec![format!("{provider_type}-alias")],
                source_kind: scryer_plugin_sdk::IndexerSourceKind::Generic,
                capabilities: Default::default(),
                scoring_policies: vec![],
                config_fields: indexer_config_fields(),
                allowed_hosts: vec![],
                rate_limit_seconds: None,
            })
        }
        "usenet_indexer" => {
            scryer_plugin_sdk::ProviderDescriptor::Indexer(scryer_plugin_sdk::IndexerDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: vec![format!("{provider_type}-alias")],
                source_kind: scryer_plugin_sdk::IndexerSourceKind::Usenet,
                capabilities: Default::default(),
                scoring_policies: vec![],
                config_fields: indexer_config_fields(),
                allowed_hosts: vec![],
                rate_limit_seconds: None,
            })
        }
        "torrent_indexer" => {
            scryer_plugin_sdk::ProviderDescriptor::Indexer(scryer_plugin_sdk::IndexerDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: vec![format!("{provider_type}-alias")],
                source_kind: scryer_plugin_sdk::IndexerSourceKind::Torrent,
                capabilities: Default::default(),
                scoring_policies: vec![],
                config_fields: indexer_config_fields(),
                allowed_hosts: vec![],
                rate_limit_seconds: None,
            })
        }
        "subtitle_provider" => {
            scryer_plugin_sdk::ProviderDescriptor::Subtitle(scryer_plugin_sdk::SubtitleDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: vec![format!("{provider_type}-alias")],
                config_fields: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                capabilities: scryer_plugin_sdk::SubtitleCapabilities::default(),
            })
        }
        "notification" => scryer_plugin_sdk::ProviderDescriptor::Notification(
            scryer_plugin_sdk::NotificationDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: vec![format!("{provider_type}-alias")],
                config_fields: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                capabilities: scryer_plugin_sdk::NotificationCapabilities::default(),
            },
        ),
        "download_client" => scryer_plugin_sdk::ProviderDescriptor::DownloadClient(
            scryer_plugin_sdk::DownloadClientDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: vec![format!("{provider_type}-alias")],
                config_fields: vec![],
                default_base_url: None,
                allowed_hosts: vec![],
                accepted_inputs: vec![],
                isolation_modes: vec![],
                capabilities: scryer_plugin_sdk::DownloadClientCapabilities::default(),
            },
        ),
        other => panic!("unsupported plugin type for runtime load helper: {other}"),
    };

    RuntimePluginLoad {
        descriptor: scryer_plugin_sdk::PluginDescriptor {
            id: plugin_id.to_string(),
            name: format!("{plugin_id} Plugin"),
            version: "0.1.0".to_string(),
            sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            socket_permissions: vec![],
            provider,
        },
        wasm_bytes: vec![1, 2, 3, 4],
        first_party: true,
    }
}

fn fixture_source_repo(plugin_id: &str) -> String {
    format!(
        "https://github.com/scryer-media/test-plugin-{}",
        plugin_id.replace('_', "-")
    )
}

fn fixture_child_catalog_url(github_repo: &GitHubRepo, version: Option<&str>) -> String {
    format!(
        "{}v{}/catalog-v2.min.json.zst",
        github_repo.release_asset_prefix(),
        version.unwrap_or("0.1.0")
    )
}

fn make_catalog_fixture_json(entries: &[serde_json::Value]) -> String {
    serde_json::json!({
        "plugins": entries
    })
    .to_string()
}

fn assert_not_available_from_catalog(err: AppError, plugin_id: &str) {
    assert!(matches!(err, AppError::NotFound(_)));
    match err {
        AppError::NotFound(msg) => {
            assert!(msg.contains(plugin_id));
            assert!(msg.contains("plugin catalog"));
        }
        _ => panic!("expected NotFound"),
    }
}

fn downloaded_release_contract(
    version: &str,
    sdk_constraint: &str,
    scryer_constraint: Option<&str>,
) -> DownloadedPluginReleaseContract {
    DownloadedPluginReleaseContract {
        version: version.to_string(),
        sdk_version: Some(scryer_plugin_sdk::SDK_VERSION.to_string()),
        sdk_constraint: sdk_constraint.to_string(),
        scryer_constraint: scryer_constraint.map(str::to_string),
    }
}

fn catalog_entry(
    id: &str,
    version: &str,
    builtin: bool,
    wasm_url: Option<&str>,
) -> serde_json::Value {
    let mut release = serde_json::json!({
        "version": version,
        "sdk_constraint": scryer_plugin_sdk::current_sdk_constraint(),
        "builtin": builtin,
    });
    if let Some(url) = wasm_url {
        release["wasm_url"] = serde_json::json!(url);
    }

    serde_json::json!({
        "id": id,
        "name": format!("{id} Plugin"),
        "description": format!("Description for {id}"),
        "plugin_type": "indexer",
        "provider_type": id,
        "official": true,
        "releases": [release],
    })
}

fn catalog_entry_with_type(
    id: &str,
    plugin_type: &str,
    version: &str,
    builtin: bool,
    wasm_url: Option<&str>,
) -> serde_json::Value {
    let mut entry = catalog_entry(id, version, builtin, wasm_url);
    entry["plugin_type"] = serde_json::json!(plugin_type);
    entry
}

fn catalog_entry_with_sdk_constraint(
    id: &str,
    version: &str,
    builtin: bool,
    wasm_url: Option<&str>,
    sdk_constraint: &str,
) -> serde_json::Value {
    let mut entry = catalog_entry(id, version, builtin, wasm_url);
    entry["releases"][0]["sdk_constraint"] = serde_json::json!(sdk_constraint);
    entry
}

fn make_indexer_config(provider_type: &str) -> IndexerConfig {
    let now = Utc::now();
    IndexerConfig {
        id: scryer_domain::Id::new().0,
        name: format!("{provider_type} config"),
        provider_type: provider_type.to_string(),
        base_url: "https://example.com".to_string(),
        api_key_encrypted: None,
        is_enabled: true,
        enable_interactive_search: true,
        enable_auto_search: true,
        rate_limit_seconds: None,
        rate_limit_burst: None,
        disabled_until: None,
        managed_parent_config_id: None,
        managed_child_key: None,
        managed_metadata_json: None,
        last_health_status: None,
        last_error_at: None,
        config_json: None,
        created_at: now,
        updated_at: now,
    }
}

struct TestHarness {
    app: AppUseCase,
    plugin_repo: Arc<MockPluginInstallationRepo>,
    indexer_config_repo: Arc<MockIndexerConfigRepo>,
    plugin_provider: Option<Arc<MockPluginProvider>>,
}

fn bootstrap_plugins(provider: Option<MockPluginProvider>) -> TestHarness {
    use crate::null_repositories::NullSettingsRepository;
    use crate::null_repositories::test_nulls::*;
    use crate::types::JwtAuthConfig;

    let plugin_repo = Arc::new(MockPluginInstallationRepo::new());
    let indexer_config_repo = Arc::new(MockIndexerConfigRepo::new());

    let mut services = AppServices::builder(
        Arc::new(NullTitleRepository),
        Arc::new(NullShowRepository),
        Arc::new(NullUserRepository),
        indexer_config_repo.clone() as Arc<dyn IndexerConfigRepository>,
        Arc::new(NullIndexerClient),
        Arc::new(NullDownloadClient),
        Arc::new(NullDownloadClientConfigRepository),
        Arc::new(NullReleaseAttemptRepository),
        Arc::new(NullSettingsRepository),
        Arc::new(NullQualityProfileRepository),
        String::new(),
    )
    .with_plugin_installations(plugin_repo.clone());
    let plugin_provider = provider.map(Arc::new);
    if let Some(provider) = &plugin_provider {
        services = services.with_plugin_provider(provider.clone());
    }
    let services = services.build_partial_for_tests();

    let registry = FacetRegistry::new();
    let app = AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    TestHarness {
        app,
        plugin_repo,
        indexer_config_repo,
        plugin_provider,
    }
}

fn bootstrap_plugins_with_subtitles(
    provider: Option<MockPluginProvider>,
    subtitle_provider: Option<Arc<dyn SubtitlePluginProvider>>,
) -> TestHarness {
    use crate::null_repositories::NullSettingsRepository;
    use crate::null_repositories::test_nulls::*;
    use crate::types::JwtAuthConfig;

    let plugin_repo = Arc::new(MockPluginInstallationRepo::new());
    let indexer_config_repo = Arc::new(MockIndexerConfigRepo::new());

    let mut services = AppServices::builder(
        Arc::new(NullTitleRepository),
        Arc::new(NullShowRepository),
        Arc::new(NullUserRepository),
        indexer_config_repo.clone() as Arc<dyn IndexerConfigRepository>,
        Arc::new(NullIndexerClient),
        Arc::new(NullDownloadClient),
        Arc::new(NullDownloadClientConfigRepository),
        Arc::new(NullReleaseAttemptRepository),
        Arc::new(NullSettingsRepository),
        Arc::new(NullQualityProfileRepository),
        String::new(),
    )
    .with_plugin_installations(plugin_repo.clone());
    let plugin_provider = provider.map(Arc::new);
    if let Some(provider) = &plugin_provider {
        services = services.with_plugin_provider(provider.clone());
    }
    if let Some(subtitle_provider) = subtitle_provider {
        services = services.with_subtitle_plugin_provider(subtitle_provider);
    }

    let registry = FacetRegistry::new();
    let app = AppUseCase::new(
        services.build_partial_for_tests(),
        JwtAuthConfig {
            issuer: "test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    TestHarness {
        app,
        plugin_repo,
        indexer_config_repo,
        plugin_provider,
    }
}

fn bootstrap_plugins_with_runtime_providers(
    provider: Option<MockPluginProvider>,
    subtitle_provider: Option<Arc<MockSubtitlePluginProvider>>,
    download_client_plugin_provider: Option<Arc<MockDownloadClientPluginProvider>>,
    notification_plugin_provider: Option<Arc<MockNotificationPluginProvider>>,
) -> TestHarness {
    use crate::null_repositories::NullSettingsRepository;
    use crate::null_repositories::test_nulls::*;
    use crate::types::JwtAuthConfig;

    let plugin_repo = Arc::new(MockPluginInstallationRepo::new());
    let indexer_config_repo = Arc::new(MockIndexerConfigRepo::new());

    let mut services = AppServices::builder(
        Arc::new(NullTitleRepository),
        Arc::new(NullShowRepository),
        Arc::new(NullUserRepository),
        indexer_config_repo.clone() as Arc<dyn IndexerConfigRepository>,
        Arc::new(NullIndexerClient),
        Arc::new(NullDownloadClient),
        Arc::new(NullDownloadClientConfigRepository),
        Arc::new(NullReleaseAttemptRepository),
        Arc::new(NullSettingsRepository),
        Arc::new(NullQualityProfileRepository),
        String::new(),
    )
    .with_plugin_installations(plugin_repo.clone());
    let plugin_provider = provider.map(Arc::new);
    if let Some(provider) = &plugin_provider {
        services = services.with_plugin_provider(provider.clone());
    }
    if let Some(provider) = &subtitle_provider {
        services = services.with_subtitle_plugin_provider(provider.clone());
    }
    if let Some(provider) = &download_client_plugin_provider {
        services = services.with_download_client_plugin_provider(provider.clone());
    }
    if let Some(provider) = &notification_plugin_provider {
        services = services.with_notification_provider(provider.clone());
    }

    let registry = FacetRegistry::new();
    let app = AppUseCase::new(
        services.build_partial_for_tests(),
        JwtAuthConfig {
            issuer: "test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    TestHarness {
        app,
        plugin_repo,
        indexer_config_repo,
        plugin_provider,
    }
}

struct MockSubtitlePluginProvider {
    builtin_types: Vec<String>,
    disabled_builtin_types: StdArc<StdMutex<Vec<String>>>,
    upsert_count: AtomicUsize,
    remove_count: AtomicUsize,
    restore_count: AtomicUsize,
    reload_count: AtomicUsize,
}

impl MockSubtitlePluginProvider {
    fn new(builtin_types: &[&str]) -> Self {
        Self {
            builtin_types: builtin_types
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            disabled_builtin_types: StdArc::new(StdMutex::new(vec![])),
            upsert_count: AtomicUsize::new(0),
            remove_count: AtomicUsize::new(0),
            restore_count: AtomicUsize::new(0),
            reload_count: AtomicUsize::new(0),
        }
    }
}

impl SubtitlePluginProvider for MockSubtitlePluginProvider {
    fn client_for_config(
        &self,
        _config: &SubtitleProviderConfig,
        _host_bindings: &HashMap<PluginHostBindingId, String>,
    ) -> Option<Arc<dyn SubtitleProviderClient>> {
        None
    }

    fn available_provider_types(&self) -> Vec<String> {
        let disabled = self
            .disabled_builtin_types
            .lock()
            .expect("disabled subtitle builtin types lock");
        self.builtin_types
            .iter()
            .filter(|provider_type| !disabled.iter().any(|value| value == *provider_type))
            .cloned()
            .collect()
    }

    fn builtin_provider_types(&self) -> Vec<String> {
        self.available_provider_types()
    }

    fn plugin_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        Some("0.1.0".to_string())
    }

    fn plugin_sdk_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        Some(scryer_plugin_sdk::SDK_VERSION.to_string())
    }

    fn plugin_sdk_constraint_for_provider(&self, _provider_type: &str) -> Option<String> {
        Some(scryer_plugin_sdk::current_sdk_constraint())
    }

    fn supports_catalog_search_for_provider(&self, _provider_type: &str) -> bool {
        false
    }

    fn recommended_facets_for_provider(&self, _provider_type: &str) -> Vec<String> {
        vec![]
    }

    fn config_fields_for_provider(
        &self,
        _provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        vec![]
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        Some(provider_type.to_string())
    }

    fn upsert_runtime_plugin(&self, _plugin: RuntimePluginLoad) -> Result<(), String> {
        self.upsert_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn remove_runtime_plugin(&self, provider_type: &str) -> Result<(), String> {
        if self
            .builtin_types
            .iter()
            .any(|builtin| builtin == provider_type)
        {
            let mut disabled = self
                .disabled_builtin_types
                .lock()
                .expect("disabled subtitle builtin types lock");
            if !disabled.iter().any(|value| value == provider_type) {
                disabled.push(provider_type.to_string());
            }
        }
        self.remove_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn restore_builtin_plugin(&self, provider_type: &str) -> Result<(), String> {
        let mut disabled = self
            .disabled_builtin_types
            .lock()
            .expect("disabled subtitle builtin types lock");
        disabled.retain(|value| value != provider_type);
        self.restore_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn reload_plugins(
        &self,
        _external_wasm_bytes: &[ExternalPluginWasm<'_>],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn reload_runtime_plugins(
        &self,
        _runtime_plugins: &[RuntimePluginLoad],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

struct MockDownloadClientPluginProvider {
    available_types: Vec<String>,
    upsert_count: AtomicUsize,
    remove_count: AtomicUsize,
    reload_count: AtomicUsize,
}

impl MockDownloadClientPluginProvider {
    fn new(provider_types: &[&str]) -> Self {
        Self {
            available_types: provider_types
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            upsert_count: AtomicUsize::new(0),
            remove_count: AtomicUsize::new(0),
            reload_count: AtomicUsize::new(0),
        }
    }
}

impl DownloadClientPluginProvider for MockDownloadClientPluginProvider {
    fn client_for_config(&self, _config: &DownloadClientConfig) -> Option<Arc<dyn DownloadClient>> {
        None
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.available_types.clone()
    }

    fn plugin_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        Some("0.1.0".to_string())
    }

    fn plugin_sdk_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        Some(scryer_plugin_sdk::SDK_VERSION.to_string())
    }

    fn plugin_sdk_constraint_for_provider(&self, _provider_type: &str) -> Option<String> {
        Some(scryer_plugin_sdk::current_sdk_constraint())
    }

    fn config_fields_for_provider(
        &self,
        _provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        vec![]
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        Some(provider_type.to_string())
    }

    fn default_base_url_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }

    fn accepted_inputs_for_provider(&self, _provider_type: &str) -> Vec<String> {
        vec![]
    }

    fn upsert_runtime_plugin(&self, _plugin: RuntimePluginLoad) -> Result<(), String> {
        self.upsert_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn remove_runtime_plugin(&self, _provider_type: &str) -> Result<(), String> {
        self.remove_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn reload_plugins(
        &self,
        _external_wasm_bytes: &[ExternalPluginWasm<'_>],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn reload_runtime_plugins(
        &self,
        _runtime_plugins: &[RuntimePluginLoad],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

struct MockNotificationPluginProvider {
    available_types: Vec<String>,
    upsert_count: AtomicUsize,
    remove_count: AtomicUsize,
    reload_count: AtomicUsize,
}

impl MockNotificationPluginProvider {
    fn new(provider_types: &[&str]) -> Self {
        Self {
            available_types: provider_types
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            upsert_count: AtomicUsize::new(0),
            remove_count: AtomicUsize::new(0),
            reload_count: AtomicUsize::new(0),
        }
    }
}

impl NotificationPluginProvider for MockNotificationPluginProvider {
    fn client_for_channel(
        &self,
        _config: &NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>> {
        None
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.available_types.clone()
    }

    fn plugin_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        Some("0.1.0".to_string())
    }

    fn plugin_sdk_version_for_provider(&self, _provider_type: &str) -> Option<String> {
        Some(scryer_plugin_sdk::SDK_VERSION.to_string())
    }

    fn plugin_sdk_constraint_for_provider(&self, _provider_type: &str) -> Option<String> {
        Some(scryer_plugin_sdk::current_sdk_constraint())
    }

    fn config_fields_for_provider(
        &self,
        _provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        vec![]
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        Some(provider_type.to_string())
    }

    fn upsert_runtime_plugin(&self, _plugin: RuntimePluginLoad) -> Result<(), String> {
        self.upsert_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn remove_runtime_plugin(&self, _provider_type: &str) -> Result<(), String> {
        self.remove_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn reload_plugins(
        &self,
        _external_wasm_bytes: &[ExternalPluginWasm<'_>],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn reload_runtime_plugins(
        &self,
        _runtime_plugins: &[RuntimePluginLoad],
        _disabled_builtins: &[String],
    ) -> Result<(), String> {
        self.reload_count.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

// ── Runtime mutation routing ────────────────────────────────────────────────

#[tokio::test]
async fn apply_runtime_plugin_upsert_routes_usenet_indexer_to_indexer_family_only() {
    let subtitle = Arc::new(MockSubtitlePluginProvider::new(&[]));
    let download = Arc::new(MockDownloadClientPluginProvider::new(&[]));
    let notification = Arc::new(MockNotificationPluginProvider::new(&[]));
    let h = bootstrap_plugins_with_runtime_providers(
        Some(MockPluginProvider::new().with_provider("animetosho", "AnimeTosho", None)),
        Some(subtitle.clone()),
        Some(download.clone()),
        Some(notification.clone()),
    );

    h.app
        .apply_runtime_plugin_upsert(
            &make_installation_with_type(
                "animetosho",
                "0.1.0",
                "usenet_indexer",
                "animetosho",
                false,
                true,
            ),
            make_runtime_plugin_load("animetosho", "usenet_indexer", "animetosho"),
        )
        .unwrap();

    let indexer = h.plugin_provider.as_ref().expect("indexer provider");
    assert_eq!(indexer.upsert_count.load(Ordering::Relaxed), 1);
    assert_eq!(indexer.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(subtitle.upsert_count.load(Ordering::Relaxed), 0);
    assert_eq!(subtitle.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(download.upsert_count.load(Ordering::Relaxed), 0);
    assert_eq!(download.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(notification.upsert_count.load(Ordering::Relaxed), 0);
    assert_eq!(notification.reload_count.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn apply_runtime_plugin_replace_removes_previous_indexer_provider_when_key_changes() {
    let subtitle = Arc::new(MockSubtitlePluginProvider::new(&[]));
    let download = Arc::new(MockDownloadClientPluginProvider::new(&[]));
    let notification = Arc::new(MockNotificationPluginProvider::new(&[]));
    let h = bootstrap_plugins_with_runtime_providers(
        Some(MockPluginProvider::new().with_provider("animetosho", "AnimeTosho", None)),
        Some(subtitle),
        Some(download),
        Some(notification),
    );

    let previous = make_installation_with_type(
        "animetosho",
        "0.1.0",
        "usenet_indexer",
        "animetosho",
        false,
        true,
    );
    let mut next = previous.clone();
    next.provider_type = "animetosho_v2".to_string();

    h.app
        .apply_runtime_plugin_replace(
            &previous,
            &next,
            make_runtime_plugin_load("animetosho", "usenet_indexer", "animetosho_v2"),
        )
        .unwrap();

    let indexer = h.plugin_provider.as_ref().expect("indexer provider");
    assert_eq!(indexer.remove_count.load(Ordering::Relaxed), 1);
    assert_eq!(indexer.upsert_count.load(Ordering::Relaxed), 1);
    assert_eq!(indexer.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(
        *indexer
            .removed_provider_types
            .lock()
            .expect("removed provider types lock"),
        vec!["animetosho".to_string()]
    );
}

#[tokio::test]
async fn apply_runtime_plugin_upsert_routes_subtitle_family_only() {
    let subtitle = Arc::new(MockSubtitlePluginProvider::new(&[]));
    let download = Arc::new(MockDownloadClientPluginProvider::new(&[]));
    let notification = Arc::new(MockNotificationPluginProvider::new(&[]));
    let h = bootstrap_plugins_with_runtime_providers(
        Some(MockPluginProvider::new().with_provider("animetosho", "AnimeTosho", None)),
        Some(subtitle.clone()),
        Some(download.clone()),
        Some(notification.clone()),
    );

    h.app
        .apply_runtime_plugin_upsert(
            &make_installation_with_type(
                "jimaku",
                "0.1.0",
                "subtitle_provider",
                "jimaku",
                false,
                true,
            ),
            make_runtime_plugin_load("jimaku", "subtitle_provider", "jimaku"),
        )
        .unwrap();

    let indexer = h.plugin_provider.as_ref().expect("indexer provider");
    assert_eq!(subtitle.upsert_count.load(Ordering::Relaxed), 1);
    assert_eq!(subtitle.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(indexer.upsert_count.load(Ordering::Relaxed), 0);
    assert_eq!(indexer.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(download.upsert_count.load(Ordering::Relaxed), 0);
    assert_eq!(download.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(notification.upsert_count.load(Ordering::Relaxed), 0);
    assert_eq!(notification.reload_count.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn apply_runtime_plugin_upsert_routes_notification_family_only() {
    let subtitle = Arc::new(MockSubtitlePluginProvider::new(&[]));
    let download = Arc::new(MockDownloadClientPluginProvider::new(&[]));
    let notification = Arc::new(MockNotificationPluginProvider::new(&[]));
    let h = bootstrap_plugins_with_runtime_providers(
        Some(MockPluginProvider::new().with_provider("animetosho", "AnimeTosho", None)),
        Some(subtitle.clone()),
        Some(download.clone()),
        Some(notification.clone()),
    );

    h.app
        .apply_runtime_plugin_upsert(
            &make_installation_with_type("email", "0.1.0", "notification", "email", false, true),
            make_runtime_plugin_load("email", "notification", "email"),
        )
        .unwrap();

    let indexer = h.plugin_provider.as_ref().expect("indexer provider");
    assert_eq!(notification.upsert_count.load(Ordering::Relaxed), 1);
    assert_eq!(notification.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(indexer.upsert_count.load(Ordering::Relaxed), 0);
    assert_eq!(indexer.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(subtitle.upsert_count.load(Ordering::Relaxed), 0);
    assert_eq!(subtitle.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(download.upsert_count.load(Ordering::Relaxed), 0);
    assert_eq!(download.reload_count.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn apply_runtime_plugin_upsert_routes_download_client_family_only() {
    let subtitle = Arc::new(MockSubtitlePluginProvider::new(&[]));
    let download = Arc::new(MockDownloadClientPluginProvider::new(&[]));
    let notification = Arc::new(MockNotificationPluginProvider::new(&[]));
    let h = bootstrap_plugins_with_runtime_providers(
        Some(MockPluginProvider::new().with_provider("animetosho", "AnimeTosho", None)),
        Some(subtitle.clone()),
        Some(download.clone()),
        Some(notification.clone()),
    );

    h.app
        .apply_runtime_plugin_upsert(
            &make_installation_with_type(
                "qbittorrent",
                "0.1.0",
                "download_client",
                "qbittorrent",
                false,
                true,
            ),
            make_runtime_plugin_load("qbittorrent", "download_client", "qbittorrent"),
        )
        .unwrap();

    let indexer = h.plugin_provider.as_ref().expect("indexer provider");
    assert_eq!(download.upsert_count.load(Ordering::Relaxed), 1);
    assert_eq!(download.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(indexer.upsert_count.load(Ordering::Relaxed), 0);
    assert_eq!(indexer.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(subtitle.upsert_count.load(Ordering::Relaxed), 0);
    assert_eq!(subtitle.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(notification.upsert_count.load(Ordering::Relaxed), 0);
    assert_eq!(notification.reload_count.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn toggle_builtin_plugin_uses_single_family_runtime_mutation() {
    let subtitle = Arc::new(MockSubtitlePluginProvider::new(&[]));
    let download = Arc::new(MockDownloadClientPluginProvider::new(&[]));
    let notification = Arc::new(MockNotificationPluginProvider::new(&[]));
    let h = bootstrap_plugins_with_runtime_providers(
        Some(MockPluginProvider::new().with_builtin_provider("nzbgeek", "NZBGeek", None)),
        Some(subtitle.clone()),
        Some(download.clone()),
        Some(notification.clone()),
    );
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("nzbgeek", "0.2.0", true, true));

    h.app
        .toggle_plugin(&config_admin(), "nzbgeek", false)
        .await
        .expect("disable builtin plugin");

    let indexer = h.plugin_provider.as_ref().expect("indexer provider");
    assert_eq!(indexer.remove_count.load(Ordering::Relaxed), 1);
    assert_eq!(indexer.restore_count.load(Ordering::Relaxed), 0);
    assert_eq!(indexer.reload_count.load(Ordering::Relaxed), 0);
    assert!(
        !indexer
            .available_provider_types()
            .iter()
            .any(|value| value == "nzbgeek")
    );
    assert_eq!(subtitle.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(download.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(notification.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(
        h.plugin_repo
            .get_enabled_payload_calls
            .load(Ordering::Relaxed),
        0
    );
    assert_eq!(
        h.plugin_repo
            .get_single_payload_calls
            .load(Ordering::Relaxed),
        0
    );

    h.app
        .toggle_plugin(&config_admin(), "nzbgeek", true)
        .await
        .expect("enable builtin plugin");

    assert_eq!(indexer.restore_count.load(Ordering::Relaxed), 1);
    assert_eq!(indexer.reload_count.load(Ordering::Relaxed), 0);
    assert!(
        indexer
            .available_provider_types()
            .iter()
            .any(|value| value == "nzbgeek")
    );
    assert_eq!(
        h.plugin_repo
            .get_enabled_payload_calls
            .load(Ordering::Relaxed),
        0
    );
    assert_eq!(
        h.plugin_repo
            .get_single_payload_calls
            .load(Ordering::Relaxed),
        0
    );
}

#[tokio::test]
async fn uninstall_notification_plugin_touches_only_notification_family() {
    let subtitle = Arc::new(MockSubtitlePluginProvider::new(&[]));
    let download = Arc::new(MockDownloadClientPluginProvider::new(&[]));
    let notification = Arc::new(MockNotificationPluginProvider::new(&["email"]));
    let h = bootstrap_plugins_with_runtime_providers(
        Some(MockPluginProvider::new().with_provider("animetosho", "AnimeTosho", None)),
        Some(subtitle.clone()),
        Some(download.clone()),
        Some(notification.clone()),
    );
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation_with_type(
            "email",
            "0.1.0",
            "notification",
            "email",
            false,
            true,
        ));

    h.app
        .uninstall_plugin(&config_admin(), "email")
        .await
        .expect("uninstall notification plugin");

    let indexer = h.plugin_provider.as_ref().expect("indexer provider");
    assert!(
        h.plugin_repo
            .get_plugin_installation("email")
            .await
            .expect("read installation")
            .is_none()
    );
    assert_eq!(notification.remove_count.load(Ordering::Relaxed), 1);
    assert_eq!(notification.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(indexer.remove_count.load(Ordering::Relaxed), 0);
    assert_eq!(indexer.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(subtitle.remove_count.load(Ordering::Relaxed), 0);
    assert_eq!(subtitle.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(download.remove_count.load(Ordering::Relaxed), 0);
    assert_eq!(download.reload_count.load(Ordering::Relaxed), 0);
    assert_eq!(
        h.plugin_repo
            .get_enabled_payload_calls
            .load(Ordering::Relaxed),
        0
    );
    assert_eq!(
        h.plugin_repo
            .get_single_payload_calls
            .load(Ordering::Relaxed),
        0
    );
}

#[tokio::test]
async fn list_rule_pack_registry_reads_cached_central_catalog() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let catalog_json = serde_json::json!({
        "plugins": [],
        "rule_packs": [{
            "id": "anime-defaults",
            "name": "Anime Defaults",
            "description": "Helpful anime rules",
            "author": "scryer",
            "version": "0.1.0",
            "url": "https://github.com/scryer-media/scryer-plugins/releases/download/catalog%2Fv2/anime-scoring.json"
        }]
    })
    .to_string();
    h.plugin_repo
        .store_catalog_fixture_json(&catalog_json)
        .await
        .unwrap();

    let packs = h.app.list_rule_pack_registry(&admin()).await.unwrap();
    assert_eq!(packs.len(), 1);
    assert_eq!(packs[0].id, "anime-defaults");
}

// ── list_available_plugins ───────────────────────────────────────────────────

#[tokio::test]
async fn list_empty_catalog_empty_installations() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn list_catalog_entries_not_installed() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_catalog_fixture_json(&[
        catalog_entry("alpha", "1.0.0", false, Some("https://example.com/a.wasm")),
        catalog_entry("beta", "2.0.0", false, Some("https://example.com/b.wasm")),
    ]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert_eq!(result.len(), 2);
    for p in &result {
        assert!(!p.is_installed);
        assert!(!p.is_enabled);
        assert!(p.installed_version.is_none());
        assert!(!p.update_available);
    }
}

#[tokio::test]
async fn list_available_plugins_marks_install_in_progress_for_initiating_actor_only() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let initiator = admin();
    let other_actor = config_admin();
    let json = make_catalog_fixture_json(&[catalog_entry(
        "alpha",
        "1.0.0",
        false,
        Some("https://example.com/a.wasm"),
    )]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();
    h.app
        .runtime
        .plugins
        .plugin_install_orchestrator
        .begin(&initiator.id, "alpha", PluginInstallOperationKind::Install)
        .await
        .unwrap();

    let admin_result = h.app.list_available_plugins(&initiator).await.unwrap();
    let other_result = h.app.list_available_plugins(&other_actor).await.unwrap();

    let admin_plugin = admin_result
        .iter()
        .find(|plugin| plugin.id == "alpha")
        .unwrap();
    assert!(admin_plugin.install_in_progress);

    let other_plugin = other_result
        .iter()
        .find(|plugin| plugin.id == "alpha")
        .unwrap();
    assert!(!other_plugin.install_in_progress);
}

#[tokio::test]
async fn list_keeps_incompatible_catalog_entries_visible_as_blocked() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_catalog_fixture_json(&[
        catalog_entry("alpha", "1.0.0", false, Some("https://example.com/a.wasm")),
        catalog_entry_with_sdk_constraint(
            "torrent-rss",
            "1.0.0",
            false,
            Some("https://example.com/torrent-rss.wasm"),
            ">=99.0.0",
        ),
    ]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert_eq!(result.len(), 2);
    let torrent_rss = result
        .iter()
        .find(|plugin| plugin.id == "torrent-rss")
        .unwrap();
    assert!(!torrent_rss.is_installed);
    assert_eq!(
        torrent_rss.blocked_reason.as_deref(),
        Some("no_compatible_release")
    );
}

#[tokio::test]
async fn list_keeps_installed_incompatible_catalog_entries_visible() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_catalog_fixture_json(&[catalog_entry_with_sdk_constraint(
        "torrent-rss",
        "2.0.0",
        false,
        Some("https://example.com/torrent-rss.wasm"),
        ">=99.0.0",
    )]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("torrent-rss", "1.0.0", false, true));

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, "torrent-rss");
    assert!(result[0].is_installed);
    assert!(!result[0].update_available);
}

#[tokio::test]
async fn list_hides_catalog_entries_with_invalid_sdk_constraint() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_catalog_fixture_json(&[catalog_entry_with_sdk_constraint(
        "torrent-rss",
        "1.0.0",
        false,
        Some("https://example.com/torrent-rss.wasm"),
        "not-a-version-req",
    )]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert!(result.is_empty());
}

#[test]
fn installed_host_block_uses_persisted_scryer_constraint_without_catalog() {
    let mut installation = make_installation("alpha", "0.1.0", false, true);
    installation.scryer_constraint = Some(">=99.0.0".to_string());

    assert!(installation_is_host_blocked(&installation));
}

#[tokio::test]
async fn decode_persisted_plugin_wasm_payload_decompresses_and_validates_blake3() {
    let wasm_bytes = b"hello plugin payload";
    let compressed = zstd::encode_all(&wasm_bytes[..], 1).expect("compress wasm bytes");
    let mut installation = make_installation("alpha", "0.1.0", false, true);
    installation.wasm_encoding = PluginWasmEncoding::Zstd;
    installation.wasm_digest_algo = Some("blake3".to_string());
    installation.wasm_digest = Some(blake3::hash(wasm_bytes).to_hex().to_string());

    let decoded = decode_persisted_plugin_wasm_payload(
        &installation,
        &PersistedPluginWasmPayload {
            encoding: PluginWasmEncoding::Zstd,
            bytes: compressed,
        },
    )
    .await
    .expect("persisted payload should decode");

    assert_eq!(decoded, wasm_bytes);
}

#[tokio::test]
async fn decode_persisted_plugin_wasm_payload_rejects_digest_mismatch() {
    let wasm_bytes = b"hello plugin payload";
    let compressed = zstd::encode_all(&wasm_bytes[..], 1).expect("compress wasm bytes");
    let mut installation = make_installation("alpha", "0.1.0", false, true);
    installation.wasm_encoding = PluginWasmEncoding::Zstd;
    installation.wasm_digest_algo = Some("blake3".to_string());
    installation.wasm_digest = Some("deadbeef".to_string());

    let error = decode_persisted_plugin_wasm_payload(
        &installation,
        &PersistedPluginWasmPayload {
            encoding: PluginWasmEncoding::Zstd,
            bytes: compressed,
        },
    )
    .await
    .expect_err("payload digest mismatch should fail");

    assert!(error.to_string().contains("digest mismatch"));
}

#[tokio::test]
async fn list_available_prefers_specific_indexer_class_over_catalog_type() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let mut installation = make_installation("torznab", "0.1.0", false, true);
    installation.plugin_type = "torrent_indexer".to_string();
    h.plugin_repo.installations.lock().await.push(installation);

    let json = make_catalog_fixture_json(&[catalog_entry_with_type(
        "torznab",
        "indexer",
        "0.2.0",
        false,
        Some("https://example.com/torznab.wasm"),
    )]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    let torznab = result.iter().find(|p| p.id == "torznab").unwrap();
    assert_eq!(torznab.plugin_type, "torrent_indexer");
}

#[tokio::test]
async fn list_installed_and_in_catalog() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_catalog_fixture_json(&[catalog_entry(
        "alpha",
        "0.2.0",
        false,
        Some("https://example.com/a.wasm"),
    )]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.1.0", false, true));

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert_eq!(result.len(), 1);
    let p = &result[0];
    assert!(p.is_installed);
    assert!(p.is_enabled);
    assert_eq!(p.installed_version.as_deref(), Some("0.1.0"));
    assert!(p.update_available);
}

#[tokio::test]
async fn list_installed_at_latest() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_catalog_fixture_json(&[catalog_entry("alpha", "0.1.0", false, None)]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.1.0", false, true));

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert!(!result[0].update_available);
}

#[tokio::test]
async fn list_installed_ahead_of_catalog() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_catalog_fixture_json(&[catalog_entry("alpha", "0.1.0", false, None)]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.2.0", false, true));

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert!(!result[0].update_available);
}

#[tokio::test]
async fn list_installed_not_in_catalog() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("manual", "1.0.0", false, true));

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert_eq!(result.len(), 1);
    let p = &result[0];
    assert!(p.is_installed);
    assert!(!p.official);
    assert!(p.wasm_url.is_none());
    assert!(!p.update_available);
}

#[tokio::test]
async fn list_merge_both_sources() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_catalog_fixture_json(&[
        catalog_entry(
            "alpha",
            "1.0.0",
            false,
            Some("https://example.com/alpha.wasm.zst"),
        ),
        catalog_entry(
            "beta",
            "1.0.0",
            false,
            Some("https://example.com/beta.wasm.zst"),
        ),
    ]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();
    {
        let mut list = h.plugin_repo.installations.lock().await;
        list.push(make_installation("alpha", "1.0.0", false, true));
        list.push(make_installation("gamma", "1.0.0", false, false));
    }

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert_eq!(result.len(), 3);
    let alpha = result.iter().find(|p| p.id == "alpha").unwrap();
    let beta = result.iter().find(|p| p.id == "beta").unwrap();
    let gamma = result.iter().find(|p| p.id == "gamma").unwrap();
    assert!(alpha.is_installed);
    assert!(!beta.is_installed);
    assert!(gamma.is_installed);
    assert!(!gamma.official);
}

#[tokio::test]
async fn list_invalid_central_catalog_json_fallback() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .store_raw_catalog_source(
            CENTRAL_CATALOG_SOURCE_KEY,
            "central",
            Some("not valid json!!!".to_string()),
        )
        .await;

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn list_invalid_semver_no_update() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_catalog_fixture_json(&[catalog_entry("alpha", "not-a-version", false, None)]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.1.0", false, true));

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert!(!result[0].update_available);
}

#[tokio::test]
async fn plugin_update_count_matches_available_plugins() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_catalog_fixture_json(&[
        catalog_entry(
            "alpha",
            "0.2.0",
            false,
            Some("https://example.com/alpha.wasm.zst"),
        ),
        catalog_entry(
            "beta",
            "1.0.0",
            false,
            Some("https://example.com/beta.wasm.zst"),
        ),
    ]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.1.0", false, true));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("beta", "1.0.0", false, true));

    let count = h.app.plugin_update_count(&admin()).await.unwrap();

    assert_eq!(count, 1);
}

#[tokio::test]
async fn list_default_base_url_from_provider() {
    let provider = MockPluginProvider::new().with_provider(
        "animetosho",
        "AnimeTosho",
        Some("https://feed.animetosho.org"),
    );
    let h = bootstrap_plugins(Some(provider));
    let json = make_catalog_fixture_json(&[catalog_entry(
        "animetosho",
        "0.1.0",
        false,
        Some("https://example.com/animetosho.wasm.zst"),
    )]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    assert_eq!(
        result[0].default_base_url.as_deref(),
        Some("https://feed.animetosho.org")
    );
}

#[tokio::test]
async fn list_uses_application_builtin_provider_types_over_catalog_flags() {
    let provider = MockPluginProvider::new().with_builtin_provider(
        "animetosho",
        "AnimeTosho",
        Some("https://feed.animetosho.org"),
    );
    let subtitle_provider = Arc::new(MockSubtitlePluginProvider::new(&[]));
    let h = bootstrap_plugins_with_subtitles(Some(provider), Some(subtitle_provider));
    let json = make_catalog_fixture_json(&[catalog_entry_with_type(
        "animetosho",
        "indexer",
        "0.3.4",
        false,
        Some("https://example.com/animetosho.wasm.zst"),
    )]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();

    let result = h.app.list_available_plugins(&admin()).await.unwrap();

    assert!(
        result
            .iter()
            .find(|plugin| plugin.id == "animetosho")
            .unwrap()
            .builtin
    );
}

#[tokio::test]
async fn list_auth_rejects_viewer() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h.app.list_available_plugins(&viewer()).await.unwrap_err();
    assert!(matches!(err, AppError::Unauthorized(_)));
}

// ── toggle_plugin ────────────────────────────────────────────────────────────

#[tokio::test]
async fn toggle_enables_disabled_plugin() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new().with_builtin_provider(
        "alpha",
        "Alpha Plugin",
        None,
    )));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "1.0.0", true, false));

    let result = h.app.toggle_plugin(&admin(), "alpha", true).await.unwrap();
    assert!(result.is_enabled);

    let stored = h
        .plugin_repo
        .get_plugin_installation("alpha")
        .await
        .unwrap()
        .unwrap();
    assert!(stored.is_enabled);
}

#[tokio::test]
async fn toggle_disables_enabled_plugin() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "1.0.0", false, true));

    let result = h.app.toggle_plugin(&admin(), "alpha", false).await.unwrap();
    assert!(!result.is_enabled);
}

#[tokio::test]
async fn list_available_keeps_disabled_builtin_plugin_installed() {
    let provider = MockPluginProvider::new().with_builtin_provider("torznab", "Torznab", None);
    let h = bootstrap_plugins(Some(provider));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("torznab", "1.0.0", true, true));

    h.app
        .toggle_plugin(&admin(), "torznab", false)
        .await
        .unwrap();

    let result = h.app.list_available_plugins(&admin()).await.unwrap();
    let torznab = result.iter().find(|plugin| plugin.id == "torznab").unwrap();
    assert!(torznab.is_installed);
    assert!(!torznab.is_enabled);
    assert!(torznab.builtin);
    assert_eq!(torznab.source_kind.as_deref(), Some("bundled"));
}

#[tokio::test]
async fn toggle_not_found() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h
        .app
        .toggle_plugin(&admin(), "nonexistent", true)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn toggle_auth_rejects_viewer() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h
        .app
        .toggle_plugin(&viewer(), "alpha", true)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Unauthorized(_)));
}

#[test]
fn provider_catalog_families_map_known_plugin_types() {
    assert_eq!(
        provider_catalog_families_for_plugin_type("subtitle_provider"),
        vec![ProviderCatalogFamily::Subtitle]
    );
    assert_eq!(
        provider_catalog_families_for_plugin_type("notification"),
        vec![ProviderCatalogFamily::Notification]
    );
    assert_eq!(
        provider_catalog_families_for_plugin_type("download_client"),
        vec![ProviderCatalogFamily::DownloadClient]
    );
    assert_eq!(
        provider_catalog_families_for_plugin_type("indexer"),
        vec![ProviderCatalogFamily::Indexer]
    );
}

#[tokio::test]
async fn toggle_publishes_provider_catalog_change_for_subtitle_plugins() {
    let subtitle_provider = Arc::new(MockSubtitlePluginProvider::new(&["jimaku"]));
    let h =
        bootstrap_plugins_with_subtitles(Some(MockPluginProvider::new()), Some(subtitle_provider));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation_with_type(
            "jimaku",
            "1.0.0",
            "subtitle_provider",
            "jimaku",
            true,
            false,
        ));

    let mut rx = h
        .app
        .subscribe_provider_catalog_changed(&config_admin())
        .await
        .unwrap();

    h.app.toggle_plugin(&admin(), "jimaku", true).await.unwrap();

    assert_eq!(
        rx.recv().await.unwrap(),
        vec![ProviderCatalogFamily::Subtitle]
    );
}

#[tokio::test]
async fn uninstall_publishes_provider_catalog_change_for_notification_plugins() {
    let notification_provider = Arc::new(MockNotificationPluginProvider::new(&["jellyfin"]));
    let h = bootstrap_plugins_with_runtime_providers(
        Some(MockPluginProvider::new()),
        None,
        None,
        Some(notification_provider),
    );
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation_with_type(
            "jellyfin",
            "1.0.0",
            "notification",
            "jellyfin",
            false,
            true,
        ));

    let mut rx = h
        .app
        .subscribe_provider_catalog_changed(&config_admin())
        .await
        .unwrap();

    h.app.uninstall_plugin(&admin(), "jellyfin").await.unwrap();

    assert_eq!(
        rx.recv().await.unwrap(),
        vec![ProviderCatalogFamily::Notification]
    );
}

// ── uninstall_plugin ─────────────────────────────────────────────────────────

#[tokio::test]
async fn uninstall_success() {
    let provider = MockPluginProvider::new();
    let h = bootstrap_plugins(Some(provider));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "1.0.0", false, true));

    h.app.uninstall_plugin(&admin(), "alpha").await.unwrap();

    let remaining = h.plugin_repo.list_plugin_installations().await.unwrap();
    assert!(remaining.is_empty());
}

#[tokio::test]
async fn uninstall_deletes_indexer_configs() {
    let provider = MockPluginProvider::new();
    let h = bootstrap_plugins(Some(provider));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "1.0.0", false, true));
    {
        let mut configs = h.indexer_config_repo.store.lock().await;
        configs.push(make_indexer_config("alpha"));
        configs.push(make_indexer_config("alpha"));
    }

    h.app.uninstall_plugin(&admin(), "alpha").await.unwrap();

    let configs = h.indexer_config_repo.store.lock().await;
    assert!(
        configs.is_empty(),
        "indexer configs should be deleted on uninstall"
    );
}

#[tokio::test]
async fn uninstall_not_found() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h
        .app
        .uninstall_plugin(&admin(), "nonexistent")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn uninstall_builtin_rejected() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("nzbgeek", "0.2.0", true, true));

    let err = h
        .app
        .uninstall_plugin(&admin(), "nzbgeek")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
    match err {
        AppError::Validation(msg) => {
            assert!(msg.contains("disable"), "expected 'disable' hint: {msg}")
        }
        _ => panic!("expected Validation error"),
    }
}

#[tokio::test]
async fn uninstall_auth_rejects_viewer() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h
        .app
        .uninstall_plugin(&viewer(), "alpha")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Unauthorized(_)));
}

// ── install_plugin error paths ───────────────────────────────────────────────

#[tokio::test]
async fn install_catalog_not_loaded() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h.app.install_plugin(&admin(), "alpha").await.unwrap_err();
    assert_not_available_from_catalog(err, "alpha");
}

#[tokio::test]
async fn install_not_in_catalog() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_catalog_fixture_json(&[catalog_entry(
        "beta",
        "1.0.0",
        false,
        Some("https://example.com/b.wasm"),
    )]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();

    let err = h.app.install_plugin(&admin(), "alpha").await.unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn install_builtin_rejected() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_catalog_fixture_json(&[catalog_entry("nzbgeek", "0.2.0", true, None)]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();

    let err = h.app.install_plugin(&admin(), "nzbgeek").await.unwrap_err();
    assert_not_available_from_catalog(err, "nzbgeek");
}

#[tokio::test]
async fn install_no_wasm_url() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_catalog_fixture_json(&[catalog_entry("alpha", "1.0.0", false, None)]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();

    let err = h.app.install_plugin(&admin(), "alpha").await.unwrap_err();
    assert_not_available_from_catalog(err, "alpha");
}

#[tokio::test]
async fn install_rejects_incompatible_sdk_line() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let json = make_catalog_fixture_json(&[catalog_entry_with_sdk_constraint(
        "torrent-rss",
        "1.0.0",
        false,
        Some("https://example.com/torrent-rss.wasm"),
        ">=99.0.0",
    )]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();

    let err = h
        .app
        .install_plugin(&admin(), "torrent-rss")
        .await
        .unwrap_err();
    assert_not_available_from_catalog(err, "torrent-rss");
}

#[tokio::test]
async fn install_auth_rejects_viewer() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h.app.install_plugin(&viewer(), "alpha").await.unwrap_err();
    assert!(matches!(err, AppError::Unauthorized(_)));
}

// ── upgrade_plugin error paths ───────────────────────────────────────────────

#[tokio::test]
async fn upgrade_not_found() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    let err = h
        .app
        .upgrade_plugin(&admin(), "nonexistent")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn upgrade_builtin_rejected() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("nzbgeek", "0.2.0", true, true));

    let err = h.app.upgrade_plugin(&admin(), "nzbgeek").await.unwrap_err();
    assert_not_available_from_catalog(err, "nzbgeek");
}

#[tokio::test]
async fn upgrade_catalog_not_loaded() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.1.0", false, true));

    let err = h.app.upgrade_plugin(&admin(), "alpha").await.unwrap_err();
    assert_not_available_from_catalog(err, "alpha");
}

#[tokio::test]
async fn upgrade_not_in_catalog() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.1.0", false, true));
    let json = make_catalog_fixture_json(&[catalog_entry("beta", "1.0.0", false, None)]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();

    let err = h.app.upgrade_plugin(&admin(), "alpha").await.unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn upgrade_already_at_latest() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.2.0", false, true));
    let json = make_catalog_fixture_json(&[catalog_entry(
        "alpha",
        "0.2.0",
        false,
        Some("https://example.com/a.wasm"),
    )]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();

    let err = h.app.upgrade_plugin(&admin(), "alpha").await.unwrap_err();
    assert!(
        matches!(err, AppError::Validation(message) if message.contains("already at version 0.2.0"))
    );
}

#[tokio::test]
async fn upgrade_no_wasm_url() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("alpha", "0.1.0", false, true));
    let json = make_catalog_fixture_json(&[catalog_entry("alpha", "0.2.0", false, None)]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();

    let err = h.app.upgrade_plugin(&admin(), "alpha").await.unwrap_err();
    assert_not_available_from_catalog(err, "alpha");
}

#[test]
fn validate_downloaded_plugin_descriptor_rejects_invalid_allowed_hosts() {
    let release =
        downloaded_release_contract("0.2.0", &scryer_plugin_sdk::current_sdk_constraint(), None);
    let descriptor = scryer_plugin_sdk::PluginDescriptor {
        id: "alpha".to_string(),
        name: "Alpha Plugin".to_string(),
        version: "0.2.0".to_string(),
        sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
        sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
        socket_permissions: vec![],
        provider: scryer_plugin_sdk::ProviderDescriptor::Indexer(
            scryer_plugin_sdk::IndexerDescriptor {
                provider_type: "alpha".to_string(),
                provider_aliases: Vec::new(),
                source_kind: scryer_plugin_sdk::IndexerSourceKind::Generic,
                capabilities: Default::default(),
                scoring_policies: Vec::new(),
                config_fields: vec![scryer_plugin_sdk::ConfigFieldDef {
                    key: "base_url".to_string(),
                    label: "Base URL".to_string(),
                    field_type: scryer_plugin_sdk::ConfigFieldType::String,
                    required: true,
                    default_value: None,
                    value_source: scryer_plugin_sdk::ConfigFieldValueSource::User,
                    role: Some(scryer_plugin_sdk::ConfigFieldRole::ConnectionUrl),
                    host_binding: None,
                    options: vec![],
                    help_text: None,
                }],
                allowed_hosts: vec!["https://example.com".to_string()],
                rate_limit_seconds: None,
            },
        ),
    };

    let err = validate_downloaded_plugin_descriptor(
        "alpha",
        "indexer",
        "alpha",
        &release,
        &descriptor,
        true,
    )
    .unwrap_err();
    match err {
        AppError::Validation(msg) => {
            assert!(msg.contains("invalid network permission pattern"))
        }
        _ => panic!("expected Validation"),
    }
}

#[test]
fn validate_downloaded_plugin_descriptor_accepts_release_sdk_constraint_override() {
    let sdk_version = semver::Version::parse(scryer_plugin_sdk::SDK_VERSION).unwrap();
    let narrow_sdk_constraint = format!(
        ">={}.{}.0, <{}.{}.0",
        sdk_version.major,
        sdk_version.minor,
        sdk_version.major,
        sdk_version.minor + 1
    );
    let release = downloaded_release_contract("0.2.0", &narrow_sdk_constraint, None);
    let descriptor = scryer_plugin_sdk::PluginDescriptor {
        id: "jellyfin".to_string(),
        name: "Jellyfin".to_string(),
        version: "0.2.0".to_string(),
        sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
        sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
        socket_permissions: vec![],
        provider: scryer_plugin_sdk::ProviderDescriptor::Notification(
            scryer_plugin_sdk::NotificationDescriptor {
                provider_type: "jellyfin".to_string(),
                provider_aliases: Vec::new(),
                allowed_hosts: Vec::new(),
                capabilities: Default::default(),
                config_fields: Vec::new(),
                default_base_url: None,
            },
        ),
    };

    let validated = validate_downloaded_plugin_descriptor(
        "jellyfin",
        "notification",
        "jellyfin",
        &release,
        &descriptor,
        true,
    )
    .unwrap();

    assert_eq!(validated.sdk_constraint, narrow_sdk_constraint);
}

#[test]
fn validate_catalog_downloaded_plugin_descriptor_skips_release_host_compatibility_check() {
    let release = downloaded_release_contract("0.2.0", ">=99.0.0", None);
    let descriptor = scryer_plugin_sdk::PluginDescriptor {
        id: "email".to_string(),
        name: "Email".to_string(),
        version: "0.2.0".to_string(),
        sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
        sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
        socket_permissions: vec![],
        provider: scryer_plugin_sdk::ProviderDescriptor::Notification(
            scryer_plugin_sdk::NotificationDescriptor {
                provider_type: "email".to_string(),
                provider_aliases: Vec::new(),
                allowed_hosts: Vec::new(),
                capabilities: Default::default(),
                config_fields: Vec::new(),
                default_base_url: None,
            },
        ),
    };

    let validated = validate_downloaded_plugin_descriptor(
        "email",
        "notification",
        "email",
        &release,
        &descriptor,
        false,
    )
    .unwrap();

    assert_eq!(validated.sdk_constraint, ">=99.0.0");
}

#[test]
fn installation_sdk_contract_filter_uses_persisted_sdk_constraint() {
    let mut installation = make_installation("jellyfin", "0.2.0", false, true);
    installation.sdk_constraint = ">=99.0.0".to_string();

    assert!(!installation_sdk_contract_is_host_compatible(&installation));
}

#[tokio::test]
async fn upgrade_rejects_incompatible_sdk_line() {
    let h = bootstrap_plugins(Some(MockPluginProvider::new()));
    h.plugin_repo
        .installations
        .lock()
        .await
        .push(make_installation("torrent-rss", "0.1.0", false, true));
    let json = make_catalog_fixture_json(&[catalog_entry_with_sdk_constraint(
        "torrent-rss",
        "0.2.0",
        false,
        Some("https://example.com/torrent-rss.wasm"),
        ">=99.0.0",
    )]);
    h.plugin_repo
        .store_catalog_fixture_json(&json)
        .await
        .unwrap();

    let err = h
        .app
        .upgrade_plugin(&admin(), "torrent-rss")
        .await
        .unwrap_err();
    assert_not_available_from_catalog(err, "torrent-rss");
}

// ── seed_builtin_plugins ─────────────────────────────────────────────────────

#[tokio::test]
async fn seed_uses_provider_builtin_inventory() {
    let provider = MockPluginProvider::new()
        .with_builtin_provider(
            "nzbgeek",
            "NZBGeek Indexer",
            Some("https://api.nzbgeek.info"),
        )
        .with_builtin_provider("newznab", "Newznab Indexer", None)
        .with_builtin_provider("torznab", "Torznab Indexer", None);
    let subtitle_provider = Arc::new(MockSubtitlePluginProvider::new(&["jimaku"]));
    let h = bootstrap_plugins_with_subtitles(Some(provider), Some(subtitle_provider));
    h.app.seed_builtin_plugins().await.unwrap();

    let seeded = h.plugin_repo.seeded.lock().await;
    assert_eq!(seeded.len(), 4);

    let ids: Vec<&str> = seeded
        .iter()
        .map(|(id, _, _, _, _, _, _, _)| id.as_str())
        .collect();
    assert!(ids.contains(&"nzbgeek"));
    assert!(ids.contains(&"newznab"));
    assert!(ids.contains(&"torznab"));
    assert!(ids.contains(&"jimaku"));
}

#[tokio::test]
async fn rebuild_plugin_provider_seeds_builtin_installations() {
    let provider = MockPluginProvider::new()
        .with_builtin_provider(
            "nzbgeek",
            "NZBGeek Indexer",
            Some("https://api.nzbgeek.info"),
        )
        .with_builtin_provider("newznab", "Newznab Indexer", None)
        .with_builtin_provider("torznab", "Torznab Indexer", None);
    let subtitle_provider = Arc::new(MockSubtitlePluginProvider::new(&["jimaku"]));
    let h = bootstrap_plugins_with_subtitles(Some(provider), Some(subtitle_provider));

    h.app.rebuild_plugin_provider().await.unwrap();

    let installations = h.plugin_repo.list_plugin_installations().await.unwrap();
    assert_eq!(installations.len(), 4);
    assert!(
        installations
            .iter()
            .all(|installation| installation.is_builtin
                && installation.source_kind == PluginSourceKind::Bundled)
    );
}

// ── reconcile_indexer_configs ────────────────────────────────────────────────

#[tokio::test]
async fn reconcile_creates_config_for_default_url_plugin() {
    let provider = MockPluginProvider::new().with_provider(
        "animetosho",
        "AnimeTosho",
        Some("https://feed.animetosho.org"),
    );
    let h = bootstrap_plugins(Some(provider));

    h.app.reconcile_indexer_configs().await.unwrap();

    let configs = h.indexer_config_repo.store.lock().await;
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].provider_type, "animetosho");
    assert_eq!(configs[0].base_url, "https://feed.animetosho.org");
    assert!(configs[0].is_enabled);
}

#[tokio::test]
async fn reconcile_skips_when_config_exists() {
    let provider = MockPluginProvider::new().with_provider(
        "animetosho",
        "AnimeTosho",
        Some("https://feed.animetosho.org"),
    );
    let h = bootstrap_plugins(Some(provider));
    h.indexer_config_repo
        .store
        .lock()
        .await
        .push(make_indexer_config("animetosho"));

    h.app.reconcile_indexer_configs().await.unwrap();

    let configs = h.indexer_config_repo.store.lock().await;
    assert_eq!(configs.len(), 1, "should not create duplicate");
}

#[tokio::test]
async fn reconcile_skips_without_default_url() {
    let provider = MockPluginProvider::new().with_provider("newznab", "Newznab", None);
    let h = bootstrap_plugins(Some(provider));

    h.app.reconcile_indexer_configs().await.unwrap();

    let configs = h.indexer_config_repo.store.lock().await;
    assert!(configs.is_empty());
}

#[tokio::test]
async fn reconcile_noop_without_plugin_provider() {
    let h = bootstrap_plugins(None);

    h.app.reconcile_indexer_configs().await.unwrap();

    let configs = h.indexer_config_repo.store.lock().await;
    assert!(configs.is_empty());
}

fn child_catalog_for_selection_test(releases: Vec<ChildCatalogRelease>) -> ChildCatalog {
    ChildCatalog {
        schema_version: "scryer.plugin.child_catalog.v2".to_string(),
        id: "email".to_string(),
        name: "Email".to_string(),
        description: "Email notifications".to_string(),
        plugin_type: "notification".to_string(),
        provider_type: "email".to_string(),
        publisher: "scryer".to_string(),
        support_tier: PluginSupportTier::Official,
        docs_url: "https://github.com/scryer-media/scryer-plugins".to_string(),
        source_repo: "https://github.com/scryer-media/scryer-plugins".to_string(),
        releases,
    }
}

#[test]
fn latest_compatible_child_release_keeps_older_sdk_line_visible() {
    let catalog = child_catalog_for_selection_test(vec![
        ChildCatalogRelease {
            version: "0.1.0".to_string(),
            sdk_constraint: format!("={SDK_VERSION}"),
            artifact_manifest_url: "https://github.com/scryer-media/scryer-plugins/releases/download/plugins%2Femail%2Fv0.1.0/plugin.manifest.json".to_string(),
        },
        ChildCatalogRelease {
            version: "0.2.0".to_string(),
            sdk_constraint: ">=999.0.0".to_string(),
            artifact_manifest_url: "https://github.com/scryer-media/scryer-plugins/releases/download/plugins%2Femail%2Fv0.2.0/plugin.manifest.json".to_string(),
        },
    ]);

    let selected = latest_compatible_child_release(&catalog).expect("compatible release");

    assert_eq!(selected.version, "0.1.0");
}
