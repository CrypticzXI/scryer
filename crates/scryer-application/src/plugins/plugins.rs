use super::catalog::{
    CentralCatalog, CentralCatalogEntry, ChildCatalog, ChildCatalogRelease, GitHubRepo,
    PluginReleaseManifest, RequiredSigner, RulePackCatalogEntry, blake3_digest, compress_zstd,
    decompress_zstd, github_outage_status_from_summary, parse_and_validate_central_catalog,
    parse_and_validate_child_catalog, parse_and_validate_release_manifest, parse_digest_string,
    plugin_manifest_asset_url, verify_digest, verify_signed_blob, verify_split_digest,
};
use super::*;
use crate::ProviderCatalogFamily;
use crate::ports::RuntimePluginLoad;
use chrono::Utc;
use scryer_domain::{
    PersistedPluginWasmPayload, PluginSourceKind, PluginSupportTier, PluginWasmEncoding,
};
use scryer_outbound_http::{
    OutboundHttpClient, OutboundHttpError, RateLimitRegistry, RequestPolicy, timeout_reqwest_client,
};
use scryer_plugin_sdk::{
    PluginDescriptor, SDK_VERSION, host_version_matches_constraint,
    load_plugin_descriptor_from_wasm_bytes, plugin_descriptor_sdk_constraint,
    sdk_constraint_or_legacy, validate_plugin_descriptor_host_permissions,
    validate_plugin_descriptor_sdk_contract, validate_sdk_contract,
};
use serde::{Deserialize, Serialize};
use std::{sync::LazyLock, time::Duration};
use tracing::{debug, warn};

static PLUGIN_HTTP_RATE_LIMITS: LazyLock<RateLimitRegistry> = LazyLock::new(RateLimitRegistry::new);
static DEFAULT_PLUGIN_HTTP_CLIENT: LazyLock<Result<OutboundHttpClient, String>> =
    LazyLock::new(|| build_plugin_http_client(None));
static RULE_PACK_PLUGIN_HTTP_CLIENT: LazyLock<Result<OutboundHttpClient, String>> =
    LazyLock::new(|| build_plugin_http_client(Some(Duration::from_secs(15))));
const CHILD_CATALOG_REFRESH_CONCURRENCY: usize = 4;

#[derive(Clone, Copy)]
enum PluginHttpClientProfile {
    DefaultFetch,
    RulePackFetch,
}

/// Registry plugin entry merged with local installation state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistryPlugin {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub latest_version: Option<String>,
    pub plugin_type: String,
    pub provider_type: String,
    pub author: String,
    pub official: bool,
    pub publisher: Option<String>,
    pub support_tier: PluginSupportTier,
    pub docs_url: Option<String>,
    pub source_repo: Option<String>,
    pub builtin: bool,
    pub source_url: Option<String>,
    pub source_kind: Option<String>,
    pub blocked_reason: Option<String>,
    pub wasm_url: Option<String>,
    pub wasm_sha256: Option<String>,
    pub min_scryer_version: Option<String>,
    /// Merged from local installation state.
    pub is_installed: bool,
    pub is_enabled: bool,
    pub installed_version: Option<String>,
    /// True when the registry version is newer than the installed version.
    pub update_available: bool,
    pub install_in_progress: bool,
    /// When set, installing this plugin auto-creates an IndexerConfig with this URL.
    pub default_base_url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PluginCatalogStatus {
    pub refresh_state: String,
    pub github_available: bool,
    pub last_checked_at: Option<String>,
    pub outage_message: Option<String>,
    pub blocked_actions: Vec<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ManualPluginPreview {
    pub plugin: RegistryPlugin,
    pub github_repo_url: String,
}

#[derive(Clone)]
struct PluginInstallProgressReporter {
    orchestrator: crate::services::PluginInstallOrchestrator,
    actor_user_id: String,
    plugin_id: String,
}

impl PluginInstallProgressReporter {
    fn new(app: &AppUseCase, actor_user_id: &str, plugin_id: &str) -> Self {
        Self {
            orchestrator: app.runtime.plugins.plugin_install_orchestrator.clone(),
            actor_user_id: actor_user_id.to_string(),
            plugin_id: plugin_id.to_string(),
        }
    }

    async fn downloading(&self) {
        self.transition(PluginInstallState::Downloading, None, None)
            .await;
    }

    async fn verifying(&self) {
        self.transition(PluginInstallState::Verifying, None, None)
            .await;
    }

    async fn installing(&self) {
        self.transition(PluginInstallState::Installing, None, None)
            .await;
    }

    async fn succeeded(&self) {
        self.transition(PluginInstallState::Succeeded, None, None)
            .await;
    }

    async fn failed(&self, error: &AppError) {
        self.transition(PluginInstallState::Failed, None, Some(error.to_string()))
            .await;
    }

    async fn transition(
        &self,
        state: PluginInstallState,
        message: Option<String>,
        error: Option<String>,
    ) {
        self.orchestrator
            .transition(&self.actor_user_id, &self.plugin_id, state, message, error)
            .await;
    }
}

#[derive(Clone, Debug)]
struct CatalogPluginResolution {
    central: Option<CentralCatalogEntry>,
    child: ChildCatalog,
    release: ChildCatalogRelease,
    source_kind: PluginSourceKind,
    effective_support_tier: PluginSupportTier,
    github_repo: GitHubRepo,
}

/// Community rule pack entry from the official catalog.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RulePackRegistryEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub version: String,
    pub url: String,
    #[serde(default)]
    pub min_scryer_version: Option<String>,
}

impl From<RulePackCatalogEntry> for RulePackRegistryEntry {
    fn from(value: RulePackCatalogEntry) -> Self {
        Self {
            id: value.id,
            name: value.name,
            description: value.description,
            author: value.author,
            version: value.version,
            url: value.url,
            min_scryer_version: value.min_scryer_version,
        }
    }
}

/// A single rule template within a community rule pack.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RulePackTemplate {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub rego_source: String,
    #[serde(default)]
    pub applied_facets: Vec<String>,
}

/// Full rule pack JSON fetched from a URL.
#[derive(Clone, Debug, Deserialize)]
struct RulePackManifest {
    #[expect(dead_code)]
    schema_version: u32,
    #[expect(dead_code)]
    id: String,
    rules: Vec<RulePackRule>,
}

#[derive(Clone, Debug, Deserialize)]
struct RulePackRule {
    id: String,
    title: String,
    description: String,
    category: String,
    #[serde(alias = "regoSource")]
    rego_source: String,
    #[serde(default, alias = "appliedFacets")]
    applied_facets: Vec<String>,
}

#[derive(Clone, Debug)]
struct DownloadedPluginReleaseContract {
    version: String,
    sdk_version: Option<String>,
    sdk_constraint: String,
    scryer_constraint: Option<String>,
}

fn downloaded_plugin_release_scryer_constraint(
    release: &DownloadedPluginReleaseContract,
) -> Option<&str> {
    release.scryer_constraint.as_deref()
}

fn normalized_constraint(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|constraint| !constraint.is_empty())
        .map(str::to_string)
}

fn current_sdk_version() -> &'static semver::Version {
    static VERSION: LazyLock<semver::Version> = LazyLock::new(|| {
        semver::Version::parse(SDK_VERSION).expect("SDK_VERSION must be valid semver")
    });
    &VERSION
}

fn normalized_release_sdk_constraint(release: &DownloadedPluginReleaseContract) -> String {
    release.sdk_version.as_deref().map_or_else(
        || release.sdk_constraint.trim().to_string(),
        |sdk_version| sdk_constraint_or_legacy(sdk_version, &release.sdk_constraint),
    )
}

fn parse_child_release_version(
    plugin_id: &str,
    release: &ChildCatalogRelease,
) -> Option<semver::Version> {
    semver::Version::parse(release.version.trim_start_matches('v')).map_or_else(
        |error| {
            warn!(
                plugin_id,
                version = release.version.as_str(),
                error = %error,
                "skipping plugin release with invalid version"
            );
            None
        },
        Some,
    )
}

fn parse_child_release_sdk_req(
    plugin_id: &str,
    release: &ChildCatalogRelease,
) -> Option<semver::VersionReq> {
    semver::VersionReq::parse(release.sdk_constraint.trim()).map_or_else(
        |error| {
            warn!(
                plugin_id,
                version = release.version.as_str(),
                sdk_constraint = release.sdk_constraint.as_str(),
                error = %error,
                "skipping plugin release with invalid sdk_constraint"
            );
            None
        },
        Some,
    )
}

fn child_release_is_sdk_compatible(plugin_id: &str, release: &ChildCatalogRelease) -> bool {
    let Some(sdk_req) = parse_child_release_sdk_req(plugin_id, release) else {
        return false;
    };
    sdk_req.matches(current_sdk_version())
}

fn downloaded_plugin_release_is_host_compatible(
    plugin_id: &str,
    release: &DownloadedPluginReleaseContract,
) -> bool {
    let constraint = normalized_release_sdk_constraint(release);
    let sdk_req = semver::VersionReq::parse(constraint.trim()).map_or_else(
        |error| {
            warn!(
                plugin_id,
                version = release.version.as_str(),
                sdk_constraint = constraint.as_str(),
                error = %error,
                "skipping plugin release with invalid sdk_constraint"
            );
            None
        },
        Some,
    );
    let Some(sdk_req) = sdk_req else {
        return false;
    };
    if !sdk_req.matches(current_sdk_version()) {
        return false;
    }
    let Some(constraint) = downloaded_plugin_release_scryer_constraint(release) else {
        return true;
    };
    let constraint = constraint.trim();
    if constraint.is_empty() {
        return true;
    }
    match host_version_matches_constraint(CURRENT_SCRYER_VERSION, constraint) {
        Ok(matches) => matches,
        Err(error) => {
            warn!(
                plugin_id,
                version = release.version.as_str(),
                scryer_constraint = constraint,
                error = %error,
                "skipping plugin release with invalid scryer_constraint"
            );
            false
        }
    }
}

fn latest_child_release(child: &ChildCatalog) -> Option<ChildCatalogRelease> {
    child
        .releases
        .iter()
        .filter_map(|release| {
            parse_child_release_version(&child.id, release).map(|version| (version, release))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, release)| release.clone())
}

fn latest_compatible_child_release(child: &ChildCatalog) -> Option<ChildCatalogRelease> {
    child
        .releases
        .iter()
        .filter(|release| child_release_is_sdk_compatible(&child.id, release))
        .filter_map(|release| {
            parse_child_release_version(&child.id, release).map(|version| (version, release))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, release)| release.clone())
}

fn latest_host_blocked_child_release(child: &ChildCatalog) -> Option<ChildCatalogRelease> {
    let latest = latest_child_release(child)?;
    let latest_version = parse_child_release_version(&child.id, &latest)?;
    let selected = latest_compatible_child_release(child);
    match selected {
        Some(selected) => {
            let selected_version = parse_child_release_version(&child.id, &selected)?;
            if latest_version > selected_version
                && !child_release_is_sdk_compatible(&child.id, &latest)
            {
                Some(latest)
            } else {
                None
            }
        }
        None if !child_release_is_sdk_compatible(&child.id, &latest) => Some(latest),
        None => None,
    }
}

fn installed_child_release(
    child: &ChildCatalog,
    installation: &PluginInstallation,
) -> Option<ChildCatalogRelease> {
    if installation.wasm_digest.is_some() {
        return None;
    }

    child
        .releases
        .iter()
        .find(|release| {
            release.version == installation.version
                && release.sdk_constraint == installation.sdk_constraint
        })
        .cloned()
}

fn installation_is_catalog_official(installation: &PluginInstallation) -> bool {
    installation.source_kind == PluginSourceKind::Downloaded
        && installation.support_tier == PluginSupportTier::Official
        && installation.wasm_digest_algo.is_some()
        && installation.wasm_digest.is_some()
}

fn installation_is_first_party(installation: &PluginInstallation) -> bool {
    installation_is_catalog_official(installation)
}

fn catalog_resolution_is_first_party(resolved: &CatalogPluginResolution) -> bool {
    resolved.source_kind == PluginSourceKind::Downloaded
        && resolved.effective_support_tier == PluginSupportTier::Official
}

fn runtime_plugin_load_from_validated(
    descriptor: PluginDescriptor,
    wasm_bytes: Vec<u8>,
    first_party: bool,
) -> RuntimePluginLoad {
    RuntimePluginLoad {
        descriptor,
        wasm_bytes,
        first_party,
    }
}

pub const RUNTIME_PLUGIN_LOAD_CONCURRENCY: usize = 4;

fn persisted_plugin_descriptor_json(descriptor: &PluginDescriptor) -> AppResult<String> {
    serde_json::to_string(descriptor).map_err(|error| {
        AppError::Repository(format!(
            "failed to serialize plugin descriptor '{}': {error}",
            descriptor.id
        ))
    })
}

fn installation_runtime_release(
    installation: &PluginInstallation,
) -> DownloadedPluginReleaseContract {
    DownloadedPluginReleaseContract {
        version: installation.version.clone(),
        sdk_version: Some(installation.sdk_version.clone()),
        sdk_constraint: installation.sdk_constraint.clone(),
        scryer_constraint: installation.scryer_constraint.clone(),
    }
}

pub async fn decode_persisted_plugin_wasm_payload(
    installation: &PluginInstallation,
    payload: &PersistedPluginWasmPayload,
) -> AppResult<Vec<u8>> {
    let wasm_bytes = match payload.encoding {
        PluginWasmEncoding::Identity => payload.bytes.clone(),
        PluginWasmEncoding::Zstd => decompress_zstd(payload.bytes.clone()).await?,
    };
    let algorithm = installation.wasm_digest_algo.as_deref().ok_or_else(|| {
        AppError::Validation(format!(
            "plugin '{}' is missing persisted wasm digest algorithm",
            installation.plugin_id
        ))
    })?;
    let expected_digest = installation.wasm_digest.as_deref().ok_or_else(|| {
        AppError::Validation(format!(
            "plugin '{}' is missing persisted wasm digest value",
            installation.plugin_id
        ))
    })?;
    verify_split_digest(
        "persisted plugin WASM",
        algorithm,
        expected_digest,
        &wasm_bytes,
    )?;
    Ok(wasm_bytes)
}

fn parse_persisted_plugin_descriptor(
    installation: &PluginInstallation,
) -> AppResult<PluginDescriptor> {
    let descriptor_json = installation
        .descriptor_json
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::Validation(format!(
                "plugin '{}' is missing persisted descriptor_json",
                installation.plugin_id
            ))
        })?;
    serde_json::from_str(descriptor_json).map_err(|error| {
        AppError::Validation(format!(
            "plugin '{}' has invalid persisted descriptor_json: {error}",
            installation.plugin_id
        ))
    })
}

pub async fn load_runtime_plugin_from_persisted_installation_payload(
    installation: &PluginInstallation,
    payload: &PersistedPluginWasmPayload,
) -> AppResult<RuntimePluginLoad> {
    let wasm_bytes = decode_persisted_plugin_wasm_payload(installation, payload).await?;
    let descriptor = parse_persisted_plugin_descriptor(installation)?;
    let validated = validate_downloaded_plugin_descriptor(
        &installation.plugin_id,
        &installation.plugin_type,
        &installation.provider_type,
        &installation_runtime_release(installation),
        &descriptor,
        false,
    )?;
    Ok(runtime_plugin_load_from_validated(
        validated.descriptor,
        wasm_bytes,
        installation_is_first_party(installation),
    ))
}

fn installation_is_host_blocked(installation: &PluginInstallation) -> bool {
    normalized_constraint(installation.scryer_constraint.as_deref()).is_some_and(|constraint| {
        host_version_matches_constraint(CURRENT_SCRYER_VERSION, &constraint)
            .map(|matches| !matches)
            .unwrap_or(true)
    })
}

fn installation_sdk_contract_is_host_compatible(installation: &PluginInstallation) -> bool {
    match validate_sdk_contract(
        installation.plugin_id.as_str(),
        installation.sdk_version.as_str(),
        installation.sdk_constraint.as_str(),
        SDK_VERSION,
    ) {
        Ok(()) => true,
        Err(error) => {
            warn!(
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

fn validate_downloaded_plugin_descriptor(
    plugin_id: &str,
    expected_plugin_type: &str,
    expected_provider_type: &str,
    release: &DownloadedPluginReleaseContract,
    descriptor: &PluginDescriptor,
    enforce_release_host_compatibility: bool,
) -> AppResult<ValidatedDownloadedPlugin> {
    validate_plugin_descriptor_sdk_contract(descriptor, SDK_VERSION)
        .map_err(AppError::Validation)?;
    validate_plugin_descriptor_host_permissions(descriptor).map_err(AppError::Validation)?;

    if descriptor.id != plugin_id {
        return Err(AppError::Validation(format!(
            "downloaded plugin descriptor id '{}' does not match registry id '{}'",
            descriptor.id, plugin_id
        )));
    }
    if descriptor.plugin_type() != expected_plugin_type {
        return Err(AppError::Validation(format!(
            "downloaded plugin '{}' has plugin_type '{}' but registry expects '{}'",
            descriptor.id,
            descriptor.plugin_type(),
            expected_plugin_type
        )));
    }
    if normalize_provider_key(descriptor.provider_type())
        != normalize_provider_key(expected_provider_type)
    {
        return Err(AppError::Validation(format!(
            "downloaded plugin '{}' has provider_type '{}' but registry expects '{}'",
            descriptor.id,
            descriptor.provider_type(),
            expected_provider_type
        )));
    }
    if descriptor.version != release.version {
        return Err(AppError::Validation(format!(
            "downloaded plugin '{}' has version '{}' but the selected release is '{}'",
            descriptor.id, descriptor.version, release.version
        )));
    }
    if release
        .sdk_version
        .as_deref()
        .is_some_and(|sdk_version| !sdk_version.trim().is_empty())
        && release.sdk_version.as_deref() != Some(descriptor.sdk_version.as_str())
    {
        return Err(AppError::Validation(format!(
            "downloaded plugin '{}' has sdk_version '{}' but the selected release is '{}'",
            descriptor.id,
            descriptor.sdk_version,
            release.sdk_version.as_deref().unwrap_or_default()
        )));
    }
    let descriptor_sdk_constraint = plugin_descriptor_sdk_constraint(descriptor);
    let release_sdk_constraint = normalized_release_sdk_constraint(release);
    if descriptor_sdk_constraint != release_sdk_constraint {
        warn!(
            plugin_id = descriptor.id.as_str(),
            version = release.version.as_str(),
            descriptor_sdk_constraint = descriptor_sdk_constraint.as_str(),
            selected_sdk_constraint = release_sdk_constraint.as_str(),
            "downloaded plugin sdk_constraint differs from selected release metadata; using selected release constraint"
        );
    }
    if enforce_release_host_compatibility
        && !downloaded_plugin_release_is_host_compatible(plugin_id, release)
    {
        return Err(AppError::Validation(format!(
            "plugin '{}' no longer has a host-compatible release for this Scryer version",
            plugin_id
        )));
    }

    Ok(ValidatedDownloadedPlugin {
        descriptor: descriptor.clone(),
        sdk_constraint: release_sdk_constraint,
    })
}

const DEFAULT_CATALOG_URL: &str = "https://github.com/scryer-media/scryer-plugins/releases/download/catalog%2Fv2/catalog-v2.min.json.zst";
const CATALOG_URL_ENV: &str = "SCRYER_PLUGIN_CATALOG_URL";
const GITHUB_STATUS_SUMMARY_URL: &str = "https://www.githubstatus.com/api/v2/summary.json";
const CENTRAL_CATALOG_SOURCE_KEY: &str = "__central_catalog_v2";
const CATALOG_STATUS_KEY: &str = "github_distribution";
const CENTRAL_CATALOG_REPO: &str = "scryer-media/scryer-plugins";
const CENTRAL_CATALOG_WORKFLOW: &str = ".github/workflows/release-plugin.yml";
const SQLITE_PLUGIN_WASM_ZSTD_LEVEL: i32 = 3;

const LEGACY_INDEXER_PLUGIN_TYPE: &str = "indexer";
const USENET_INDEXER_PLUGIN_TYPE: &str = "usenet_indexer";
const TORRENT_INDEXER_PLUGIN_TYPE: &str = "torrent_indexer";
const CURRENT_SCRYER_VERSION: &str = env!("CARGO_PKG_VERSION");

fn current_scryer_version() -> &'static semver::Version {
    static VERSION: LazyLock<semver::Version> = LazyLock::new(|| {
        semver::Version::parse(CURRENT_SCRYER_VERSION)
            .expect("CARGO_PKG_VERSION must be a valid semver version")
    });
    &VERSION
}

fn plugin_catalog_url() -> String {
    std::env::var(CATALOG_URL_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_CATALOG_URL.to_string())
}

fn bundle_url_for(url: &str) -> String {
    format!("{url}.bundle")
}

fn child_catalog_source_key(plugin_id: &str) -> String {
    format!("child:{plugin_id}")
}

fn manual_catalog_source_key(repo: &GitHubRepo) -> String {
    format!("manual:{}", repo.slug())
}

fn is_indexer_plugin_type(plugin_type: &str) -> bool {
    matches!(
        plugin_type,
        LEGACY_INDEXER_PLUGIN_TYPE | USENET_INDEXER_PLUGIN_TYPE | TORRENT_INDEXER_PLUGIN_TYPE
    )
}

fn provider_catalog_families_for_plugin_type(plugin_type: &str) -> Vec<ProviderCatalogFamily> {
    if is_indexer_plugin_type(plugin_type) {
        return vec![ProviderCatalogFamily::Indexer];
    }

    match plugin_type {
        "download_client" => vec![ProviderCatalogFamily::DownloadClient],
        "notification" => vec![ProviderCatalogFamily::Notification],
        "subtitle_provider" => vec![ProviderCatalogFamily::Subtitle],
        _ => ProviderCatalogFamily::all().into_iter().collect(),
    }
}

fn merged_plugin_type(registry_type: &str, installed_type: Option<&str>) -> String {
    match installed_type {
        Some(installed)
            if is_indexer_plugin_type(registry_type) && is_indexer_plugin_type(installed) =>
        {
            if registry_type == LEGACY_INDEXER_PLUGIN_TYPE
                && installed != LEGACY_INDEXER_PLUGIN_TYPE
            {
                installed.to_string()
            } else {
                registry_type.to_string()
            }
        }
        _ => registry_type.to_string(),
    }
}

struct BuiltinPluginSeed {
    name: String,
    version: String,
    sdk_version: String,
    sdk_constraint: String,
    plugin_type: String,
    provider_type: String,
}

fn normalize_provider_key(provider_type: &str) -> String {
    provider_type.trim().to_ascii_lowercase()
}

fn builtin_lookup_key(plugin_type: &str, provider_type: &str) -> String {
    let family = if is_indexer_plugin_type(plugin_type) {
        "indexer"
    } else {
        plugin_type
    };
    format!("{family}::{}", normalize_provider_key(provider_type))
}

fn is_reserved_first_party_provider(provider_type: &str) -> bool {
    provider_type.trim().eq_ignore_ascii_case("prowlarr")
}

fn source_kind_label(source_kind: PluginSourceKind) -> String {
    match source_kind {
        PluginSourceKind::Bundled => "bundled".to_string(),
        PluginSourceKind::Downloaded => "downloaded".to_string(),
        PluginSourceKind::Manual => "manual".to_string(),
    }
}

#[derive(Debug)]
struct ValidatedDownloadedPlugin {
    descriptor: PluginDescriptor,
    sdk_constraint: String,
}

struct ChildCatalogRefreshOutcome {
    source: PluginCatalogSource,
}

fn plugin_type_belongs_to_indexer_family(plugin_type: &str) -> bool {
    matches!(
        plugin_type,
        "indexer" | "usenet_indexer" | "torrent_indexer"
    )
}

fn build_plugin_http_client(timeout: Option<Duration>) -> Result<OutboundHttpClient, String> {
    let client = timeout_reqwest_client(timeout)
        .map_err(|error| format!("failed to build plugin HTTP client: {error}"))?;

    Ok(OutboundHttpClient::new(
        client,
        PLUGIN_HTTP_RATE_LIMITS.clone(),
    ))
}

fn plugin_http_client(profile: PluginHttpClientProfile) -> AppResult<&'static OutboundHttpClient> {
    let cached = match profile {
        PluginHttpClientProfile::DefaultFetch => &*DEFAULT_PLUGIN_HTTP_CLIENT,
        PluginHttpClientProfile::RulePackFetch => &*RULE_PACK_PLUGIN_HTTP_CLIENT,
    };

    cached
        .as_ref()
        .map_err(|error| AppError::Repository(error.clone()))
}

fn plugin_request_policy(
    scope: impl Into<String>,
    request_label: impl Into<String>,
) -> RequestPolicy {
    RequestPolicy::safe_read(scope.into(), request_label.into())
        .with_max_retries(2)
        .with_backoff(Duration::from_secs(1), Duration::from_secs(30))
}

fn map_plugin_outbound_error(label: &str, error: OutboundHttpError) -> AppError {
    match error {
        OutboundHttpError::RateLimited(rate_limited) => AppError::Repository(
            match rate_limited.retry_after.filter(|delay| !delay.is_zero()) {
                Some(delay) => format!(
                    "failed to download {label}: rate limited, retry after {}s",
                    delay.as_secs()
                ),
                None => format!("failed to download {label}: rate limited"),
            },
        ),
        OutboundHttpError::Transport { source, .. } => {
            AppError::Repository(format!("failed to download {label}: {source}"))
        }
    }
}

async fn fetch_plugin_bytes(
    url: &str,
    label: &str,
    scope: impl Into<String>,
) -> AppResult<Vec<u8>> {
    let outbound_http = plugin_http_client(PluginHttpClientProfile::DefaultFetch)?;
    let response = outbound_http
        .send(plugin_request_policy(scope, label), || {
            outbound_http.client().get(url)
        })
        .await
        .map_err(|error| map_plugin_outbound_error(label, error))?
        .error_for_status()
        .map_err(|error| AppError::Repository(format!("failed to download {label}: {error}")))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|error| AppError::Repository(format!("failed to read {label}: {error}")))?;
    Ok(bytes.to_vec())
}

fn validate_catalog_downloaded_plugin_release(
    plugin_id: &str,
    expected_plugin_type: &str,
    expected_provider_type: &str,
    release: &DownloadedPluginReleaseContract,
    wasm_bytes: &[u8],
) -> AppResult<ValidatedDownloadedPlugin> {
    let descriptor =
        load_plugin_descriptor_from_wasm_bytes(wasm_bytes).map_err(AppError::Validation)?;
    validate_downloaded_plugin_descriptor(
        plugin_id,
        expected_plugin_type,
        expected_provider_type,
        release,
        &descriptor,
        false,
    )
}

impl AppUseCase {
    fn builtin_plugin_inventory(&self) -> Vec<BuiltinPluginSeed> {
        let mut builtins = Vec::new();

        if let Some(provider) = self.services.integrations.plugin_provider.available() {
            for provider_type in provider.builtin_provider_types() {
                let provider_key = normalize_provider_key(&provider_type);
                let Some(name) = provider.plugin_name_for_provider(&provider_key) else {
                    continue;
                };
                let Some(version) = provider.plugin_version_for_provider(&provider_key) else {
                    continue;
                };
                let Some(sdk_version) = provider.plugin_sdk_version_for_provider(&provider_key)
                else {
                    continue;
                };
                let Some(sdk_constraint) =
                    provider.plugin_sdk_constraint_for_provider(&provider_key)
                else {
                    continue;
                };
                let plugin_type = provider
                    .plugin_type_for_provider(&provider_key)
                    .unwrap_or_else(|| LEGACY_INDEXER_PLUGIN_TYPE.to_string());
                builtins.push(BuiltinPluginSeed {
                    name,
                    version,
                    sdk_version,
                    sdk_constraint,
                    plugin_type,
                    provider_type: provider_key,
                });
            }
        }

        if let Some(provider) = self
            .services
            .integrations
            .subtitle_plugin_provider
            .available()
        {
            for provider_type in provider.builtin_provider_types() {
                let provider_key = normalize_provider_key(&provider_type);
                let Some(name) = provider.plugin_name_for_provider(&provider_key) else {
                    continue;
                };
                let Some(version) = provider.plugin_version_for_provider(&provider_key) else {
                    continue;
                };
                let Some(sdk_version) = provider.plugin_sdk_version_for_provider(&provider_key)
                else {
                    continue;
                };
                let Some(sdk_constraint) =
                    provider.plugin_sdk_constraint_for_provider(&provider_key)
                else {
                    continue;
                };
                builtins.push(BuiltinPluginSeed {
                    name,
                    version,
                    sdk_version,
                    sdk_constraint,
                    plugin_type: "subtitle_provider".to_string(),
                    provider_type: provider_key,
                });
            }
        }

        if let Some(provider) = self
            .services
            .integrations
            .download_client_plugin_provider
            .available()
        {
            for provider_type in provider.builtin_provider_types() {
                let provider_key = normalize_provider_key(&provider_type);
                let Some(name) = provider.plugin_name_for_provider(&provider_key) else {
                    continue;
                };
                let Some(version) = provider.plugin_version_for_provider(&provider_key) else {
                    continue;
                };
                let Some(sdk_version) = provider.plugin_sdk_version_for_provider(&provider_key)
                else {
                    continue;
                };
                let Some(sdk_constraint) =
                    provider.plugin_sdk_constraint_for_provider(&provider_key)
                else {
                    continue;
                };
                builtins.push(BuiltinPluginSeed {
                    name,
                    version,
                    sdk_version,
                    sdk_constraint,
                    plugin_type: "download_client".to_string(),
                    provider_type: provider_key,
                });
            }
        }

        if let Some(provider) = self.services.notifications.notification_provider() {
            for provider_type in provider.builtin_provider_types() {
                let provider_key = normalize_provider_key(&provider_type);
                let Some(name) = provider.plugin_name_for_provider(&provider_key) else {
                    continue;
                };
                let Some(version) = provider.plugin_version_for_provider(&provider_key) else {
                    continue;
                };
                let Some(sdk_version) = provider.plugin_sdk_version_for_provider(&provider_key)
                else {
                    continue;
                };
                let Some(sdk_constraint) =
                    provider.plugin_sdk_constraint_for_provider(&provider_key)
                else {
                    continue;
                };
                builtins.push(BuiltinPluginSeed {
                    name,
                    version,
                    sdk_version,
                    sdk_constraint,
                    plugin_type: "notification".to_string(),
                    provider_type: provider_key,
                });
            }
        }

        builtins
    }

    fn builtin_seed_by_key(&self) -> std::collections::HashMap<String, BuiltinPluginSeed> {
        self.builtin_plugin_inventory()
            .into_iter()
            .map(|seed| {
                (
                    builtin_lookup_key(&seed.plugin_type, &seed.provider_type),
                    seed,
                )
            })
            .collect()
    }

    fn default_base_url_for_plugin(
        &self,
        plugin_type: &str,
        provider_type: &str,
    ) -> Option<String> {
        match plugin_type {
            "download_client" => self
                .services
                .integrations
                .download_client_plugin_provider
                .available()
                .and_then(|provider| provider.default_base_url_for_provider(provider_type)),
            _ if is_indexer_plugin_type(plugin_type) => self
                .services
                .integrations
                .plugin_provider
                .available()
                .and_then(|provider| provider.default_base_url_for_provider(provider_type)),
            _ => None,
        }
    }

    async fn load_runtime_plugin_for_installation(
        &self,
        installation: &PluginInstallation,
    ) -> AppResult<RuntimePluginLoad> {
        let payload = self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation_wasm_payload(&installation.plugin_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "plugin '{}' is missing persisted WASM payload",
                    installation.plugin_id
                ))
            })?;
        load_runtime_plugin_from_persisted_installation_payload(installation, &payload).await
    }

    fn apply_runtime_plugin_upsert(
        &self,
        installation: &PluginInstallation,
        runtime_plugin: RuntimePluginLoad,
    ) -> AppResult<()> {
        if is_indexer_plugin_type(&installation.plugin_type) {
            let provider = self
                .services
                .integrations
                .plugin_provider
                .available()
                .ok_or_else(|| {
                    AppError::Repository("indexer plugin provider unavailable".to_string())
                })?;
            provider
                .upsert_runtime_plugin(runtime_plugin)
                .map_err(|e| {
                    AppError::Repository(format!("failed to upsert indexer plugin: {e}"))
                })?;
            return Ok(());
        }

        match installation.plugin_type.as_str() {
            "download_client" => {
                let provider = self
                    .services
                    .integrations
                    .download_client_plugin_provider
                    .available()
                    .ok_or_else(|| {
                        AppError::Repository(
                            "download client plugin provider unavailable".to_string(),
                        )
                    })?;
                provider
                    .upsert_runtime_plugin(runtime_plugin)
                    .map_err(|e| {
                        AppError::Repository(format!(
                            "failed to upsert download client plugin: {e}"
                        ))
                    })?;
            }
            "notification" => {
                let provider = self
                    .services
                    .notifications
                    .notification_provider()
                    .ok_or_else(|| {
                        AppError::Repository("notification plugin provider unavailable".to_string())
                    })?;
                provider
                    .upsert_runtime_plugin(runtime_plugin)
                    .map_err(|e| {
                        AppError::Repository(format!("failed to upsert notification plugin: {e}"))
                    })?;
            }
            "subtitle_provider" => {
                let provider = self
                    .services
                    .integrations
                    .subtitle_plugin_provider
                    .available()
                    .ok_or_else(|| {
                        AppError::Repository("subtitle plugin provider unavailable".to_string())
                    })?;
                provider
                    .upsert_runtime_plugin(runtime_plugin)
                    .map_err(|e| {
                        AppError::Repository(format!("failed to upsert subtitle plugin: {e}"))
                    })?;
            }
            other => {
                return Err(AppError::Validation(format!(
                    "unsupported plugin_type '{}' for runtime upsert",
                    other
                )));
            }
        }

        Ok(())
    }

    fn apply_runtime_plugin_replace(
        &self,
        previous_installation: &PluginInstallation,
        next_installation: &PluginInstallation,
        runtime_plugin: RuntimePluginLoad,
    ) -> AppResult<()> {
        if previous_installation.plugin_type != next_installation.plugin_type
            || previous_installation.provider_type != next_installation.provider_type
        {
            self.apply_runtime_plugin_removal_for_values(
                previous_installation.plugin_type.as_str(),
                previous_installation.provider_type.as_str(),
            )?;
        }

        self.apply_runtime_plugin_upsert(next_installation, runtime_plugin)
    }

    fn apply_runtime_plugin_removal(&self, installation: &PluginInstallation) -> AppResult<()> {
        self.apply_runtime_plugin_removal_for_values(
            installation.plugin_type.as_str(),
            installation.provider_type.as_str(),
        )
    }

    fn apply_runtime_plugin_removal_for_values(
        &self,
        plugin_type: &str,
        provider_type: &str,
    ) -> AppResult<()> {
        if is_indexer_plugin_type(plugin_type) {
            let provider = self
                .services
                .integrations
                .plugin_provider
                .available()
                .ok_or_else(|| {
                    AppError::Repository("indexer plugin provider unavailable".to_string())
                })?;
            provider.remove_runtime_plugin(provider_type).map_err(|e| {
                AppError::Repository(format!("failed to remove indexer plugin: {e}"))
            })?;
            return Ok(());
        }

        match plugin_type {
            "download_client" => {
                let provider = self
                    .services
                    .integrations
                    .download_client_plugin_provider
                    .available()
                    .ok_or_else(|| {
                        AppError::Repository(
                            "download client plugin provider unavailable".to_string(),
                        )
                    })?;
                provider.remove_runtime_plugin(provider_type).map_err(|e| {
                    AppError::Repository(format!("failed to remove download client plugin: {e}"))
                })?;
            }
            "notification" => {
                let provider = self
                    .services
                    .notifications
                    .notification_provider()
                    .ok_or_else(|| {
                        AppError::Repository("notification plugin provider unavailable".to_string())
                    })?;
                provider.remove_runtime_plugin(provider_type).map_err(|e| {
                    AppError::Repository(format!("failed to remove notification plugin: {e}"))
                })?;
            }
            "subtitle_provider" => {
                let provider = self
                    .services
                    .integrations
                    .subtitle_plugin_provider
                    .available()
                    .ok_or_else(|| {
                        AppError::Repository("subtitle plugin provider unavailable".to_string())
                    })?;
                provider.remove_runtime_plugin(provider_type).map_err(|e| {
                    AppError::Repository(format!("failed to remove subtitle plugin: {e}"))
                })?;
            }
            other => {
                return Err(AppError::Validation(format!(
                    "unsupported plugin_type '{}' for runtime removal",
                    other
                )));
            }
        }

        Ok(())
    }

    fn apply_runtime_builtin_restore(&self, installation: &PluginInstallation) -> AppResult<()> {
        let provider_type = installation.provider_type.as_str();
        if is_indexer_plugin_type(&installation.plugin_type) {
            let provider = self
                .services
                .integrations
                .plugin_provider
                .available()
                .ok_or_else(|| {
                    AppError::Repository("indexer plugin provider unavailable".to_string())
                })?;
            provider
                .restore_builtin_plugin(provider_type)
                .map_err(|e| {
                    AppError::Repository(format!("failed to restore built-in indexer plugin: {e}"))
                })?;
            return Ok(());
        }

        match installation.plugin_type.as_str() {
            "subtitle_provider" => {
                let provider = self
                    .services
                    .integrations
                    .subtitle_plugin_provider
                    .available()
                    .ok_or_else(|| {
                        AppError::Repository("subtitle plugin provider unavailable".to_string())
                    })?;
                provider
                    .restore_builtin_plugin(provider_type)
                    .map_err(|e| {
                        AppError::Repository(format!(
                            "failed to restore built-in subtitle plugin: {e}"
                        ))
                    })?;
            }
            other => {
                return Err(AppError::Validation(format!(
                    "unsupported plugin_type '{}' for built-in restore",
                    other
                )));
            }
        }

        Ok(())
    }

    async fn finalize_runtime_plugin_mutation(
        &self,
        plugin_type: &str,
        runtime_touched: bool,
    ) -> AppResult<()> {
        self.finalize_runtime_plugin_mutation_for_types([plugin_type], runtime_touched)
            .await
    }

    async fn finalize_runtime_plugin_mutation_for_types<'a>(
        &self,
        plugin_types: impl IntoIterator<Item = &'a str>,
        runtime_touched: bool,
    ) -> AppResult<()> {
        let plugin_types = plugin_types.into_iter().collect::<Vec<_>>();
        if runtime_touched
            && plugin_types
                .iter()
                .any(|plugin_type| is_indexer_plugin_type(plugin_type))
        {
            self.rebuild_user_rules_engine().await?;
        }

        let mut families = plugin_types
            .into_iter()
            .flat_map(provider_catalog_families_for_plugin_type)
            .collect::<Vec<_>>();
        families.sort_by_key(|family| family.as_str());
        families.dedup();
        self.publish_provider_catalog_changed(families);
        Ok(())
    }

    /// Seed database rows for built-in plugins. Uses INSERT OR IGNORE so
    /// existing user toggles are preserved across restarts.
    pub async fn seed_builtin_plugins(&self) -> AppResult<()> {
        let repo = &self.services.customization.plugin_installations;
        let builtins = self.builtin_plugin_inventory();

        let builtin_keys = builtins
            .iter()
            .map(|builtin| builtin_lookup_key(&builtin.plugin_type, &builtin.provider_type))
            .collect::<std::collections::HashSet<_>>();

        for builtin in builtins {
            repo.seed_builtin(
                &builtin.provider_type,
                &builtin.name,
                "",
                &builtin.version,
                &builtin.sdk_version,
                &builtin.sdk_constraint,
                &builtin.plugin_type,
                &builtin.provider_type,
            )
            .await?;
        }

        let stale_builtin_plugin_ids = repo
            .list_plugin_installations()
            .await?
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
            repo.delete_plugin_installation(&plugin_id).await?;
        }

        Ok(())
    }

    /// Reload runtime plugin providers from database state + builtins.
    pub async fn reload_plugin_providers(&self) -> AppResult<()> {
        let enabled = self
            .services
            .customization
            .plugin_installations
            .get_enabled_plugin_wasm_bytes()
            .await?;

        let mut runtime_plugins = Vec::new();
        let mut pending_plugins = enabled.into_iter().filter_map(|(installation, payload)| {
            if !matches!(
                installation.source_kind,
                PluginSourceKind::Downloaded | PluginSourceKind::Manual
            ) {
                return None;
            }
            if !installation_sdk_contract_is_host_compatible(&installation) {
                return None;
            }
            if installation_is_host_blocked(&installation) {
                return None;
            }

            payload.map(|payload| (installation, payload))
        });
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..RUNTIME_PLUGIN_LOAD_CONCURRENCY {
            let Some((installation, payload)) = pending_plugins.next() else {
                break;
            };
            tasks.spawn(async move {
                let plugin_id = installation.plugin_id.clone();
                let version = installation.version.clone();
                let loaded = load_runtime_plugin_from_persisted_installation_payload(
                    &installation,
                    &payload,
                )
                .await;
                (plugin_id, version, loaded)
            });
        }
        while let Some(result) = tasks.join_next().await {
            let (plugin_id, version, loaded) = result.map_err(|error| {
                AppError::Repository(format!("runtime plugin load task panicked: {error}"))
            })?;
            match loaded {
                Ok(runtime_plugin) => runtime_plugins.push(runtime_plugin),
                Err(error) => {
                    warn!(
                        plugin_id = plugin_id.as_str(),
                        version = version.as_str(),
                        error = %error,
                        "skipping installed plugin after persisted payload validation failed"
                    );
                }
            }

            if let Some((installation, payload)) = pending_plugins.next() {
                tasks.spawn(async move {
                    let plugin_id = installation.plugin_id.clone();
                    let version = installation.version.clone();
                    let loaded = load_runtime_plugin_from_persisted_installation_payload(
                        &installation,
                        &payload,
                    )
                    .await;
                    (plugin_id, version, loaded)
                });
            }
        }
        let indexer_plugins = runtime_plugins
            .iter()
            .filter(|plugin| plugin_type_belongs_to_indexer_family(plugin.descriptor.plugin_type()))
            .cloned()
            .collect::<Vec<_>>();
        let download_client_plugins = runtime_plugins
            .iter()
            .filter(|plugin| plugin.descriptor.plugin_type() == "download_client")
            .cloned()
            .collect::<Vec<_>>();
        let subtitle_plugins = runtime_plugins
            .iter()
            .filter(|plugin| plugin.descriptor.plugin_type() == "subtitle_provider")
            .cloned()
            .collect::<Vec<_>>();
        let notification_plugins = runtime_plugins
            .iter()
            .filter(|plugin| plugin.descriptor.plugin_type() == "notification")
            .cloned()
            .collect::<Vec<_>>();

        // Collect provider_types of builtins the user has disabled
        // (must query all installations, not just enabled ones)
        let all_installations = self
            .services
            .customization
            .plugin_installations
            .list_plugin_installations()
            .await?;
        let disabled_builtins: Vec<String> = all_installations
            .iter()
            .filter(|inst| {
                inst.is_builtin
                    && !inst.is_enabled
                    && !is_reserved_first_party_provider(&inst.provider_type)
            })
            .map(|inst| inst.provider_type.clone())
            .collect();

        if let Some(provider) = self.services.integrations.plugin_provider.available() {
            provider
                .reload_runtime_plugins(&indexer_plugins, &disabled_builtins)
                .map_err(|e| {
                    AppError::Repository(format!("failed to reload plugin provider: {e}"))
                })?;
        }

        if let Some(provider) = self
            .services
            .integrations
            .download_client_plugin_provider
            .available()
        {
            provider
                .reload_runtime_plugins(&download_client_plugins, &disabled_builtins)
                .map_err(|e| {
                    AppError::Repository(format!(
                        "failed to reload download client plugin provider: {e}"
                    ))
                })?;
        }

        if let Some(provider) = self
            .services
            .integrations
            .subtitle_plugin_provider
            .available()
        {
            provider
                .reload_runtime_plugins(&subtitle_plugins, &disabled_builtins)
                .map_err(|e| {
                    AppError::Repository(format!("failed to reload subtitle plugin provider: {e}"))
                })?;
        }

        // Also rebuild notification plugin provider
        if let Some(notif_provider) = self.services.notifications.notification_provider() {
            notif_provider
                .reload_runtime_plugins(&notification_plugins, &disabled_builtins)
                .map_err(|e| {
                    AppError::Repository(format!(
                        "failed to reload notification plugin provider: {e}"
                    ))
                })?;
        }

        Ok(())
    }

    /// Rebuild the runtime plugin providers and rules engine from the latest plugin state.
    pub async fn rebuild_plugin_provider(&self) -> AppResult<()> {
        self.reload_plugin_providers().await?;
        self.seed_builtin_plugins().await?;
        self.rebuild_user_rules_engine().await?;
        Ok(())
    }

    /// Ensure every auto-provisionable indexer plugin with a default connection URL
    /// has at least one IndexerConfig. This covers the case where a plugin was
    /// installed before the auto-create logic existed, or when the registry was
    /// stale at install time.
    pub async fn reconcile_indexer_configs(&self) -> AppResult<()> {
        let Some(provider) = self.services.integrations.plugin_provider.available() else {
            return Ok(());
        };

        let now = Utc::now();
        for pt in provider.available_provider_types() {
            let fields = provider.config_fields_for_provider(&pt);
            let Some(connection_field) = fields
                .iter()
                .find(|field| field.role == Some(scryer_domain::ConfigFieldRole::ConnectionUrl))
            else {
                continue;
            };
            let Some(default_url) = provider.default_base_url_for_provider(&pt) else {
                continue;
            };
            if should_skip_auto_created_indexer_config(&pt) {
                continue;
            }
            let existing = self
                .services
                .integrations
                .indexer_configs
                .list(Some(pt.clone()))
                .await
                .unwrap_or_default();
            if existing.is_empty() {
                let name = provider
                    .plugin_name_for_provider(&pt)
                    .unwrap_or_else(|| pt.clone());
                let config = IndexerConfig {
                    id: Id::new().0,
                    name,
                    provider_type: pt.clone(),
                    base_url: default_url.clone(),
                    api_key_encrypted: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    rate_limit_seconds: provider.rate_limit_seconds_for_provider(&pt),
                    rate_limit_burst: None,
                    disabled_until: None,
                    managed_parent_config_id: None,
                    managed_child_key: None,
                    managed_metadata_json: None,
                    last_health_status: None,
                    last_error_at: None,
                    config_json: Some(
                        serde_json::json!({
                            connection_field.key.clone(): default_url,
                        })
                        .to_string(),
                    ),
                    created_at: now,
                    updated_at: now,
                };
                if let Err(e) = self
                    .services
                    .integrations
                    .indexer_configs
                    .create(config)
                    .await
                {
                    tracing::warn!(
                        error = %e,
                        provider_type = pt.as_str(),
                        "failed to auto-create indexer config during reconciliation"
                    );
                } else {
                    tracing::info!(
                        provider_type = pt.as_str(),
                        "auto-created indexer config for plugin"
                    );
                }
            }
        }
        Ok(())
    }

    /// Returns all available indexer provider types with their config field schemas.
    /// Tuple: (provider_type, name, config_fields, default_base_url)
    pub fn available_indexer_provider_types(
        &self,
    ) -> Vec<(
        String,
        String,
        Vec<scryer_domain::ConfigFieldDef>,
        Option<String>,
    )> {
        let Some(provider) = self.services.integrations.plugin_provider.available() else {
            return vec![];
        };
        let mut seen = std::collections::HashSet::new();
        provider
            .available_provider_types()
            .into_iter()
            .filter(|pt| seen.insert(pt.clone()))
            .map(|pt| {
                let name = provider
                    .plugin_name_for_provider(&pt)
                    .unwrap_or_else(|| pt.clone());
                let fields = provider.config_fields_for_provider(&pt);
                let default_base_url = provider.default_base_url_for_provider(&pt);
                (pt, name, fields, default_base_url)
            })
            .collect()
    }

    pub fn available_download_client_provider_types(
        &self,
    ) -> Vec<(
        String,
        String,
        Vec<scryer_domain::ConfigFieldDef>,
        Option<String>,
    )> {
        let Some(provider) = self
            .services
            .integrations
            .download_client_plugin_provider
            .available()
        else {
            return vec![];
        };
        let mut seen = std::collections::HashSet::new();
        provider
            .available_provider_types()
            .into_iter()
            .filter(|pt| seen.insert(pt.clone()))
            .map(|pt| {
                let name = provider
                    .plugin_name_for_provider(&pt)
                    .unwrap_or_else(|| pt.clone());
                let fields = provider.config_fields_for_provider(&pt);
                let default_base_url = provider.default_base_url_for_provider(&pt);
                (pt, name, fields, default_base_url)
            })
            .collect()
    }

    pub async fn test_plugin_download_client_connection(
        &self,
        actor: &User,
        client_type: &str,
        config_json: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let provider = self
            .services
            .integrations
            .download_client_plugin_provider
            .available()
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "test connection is not supported for client type '{}'",
                    client_type.trim()
                ))
            })?;

        let client_type = client_type.trim().to_lowercase();
        let config_json = config_json.trim();
        let config_json = if config_json.is_empty() {
            "{}".to_string()
        } else {
            serde_json::to_string(
                &serde_json::from_str::<serde_json::Value>(config_json).map_err(|error| {
                    AppError::Validation(format!("invalid client config_json: {error}"))
                })?,
            )
            .map_err(|error| AppError::Validation(format!("invalid client config_json: {error}")))?
        };

        let now = chrono::Utc::now();
        let config = DownloadClientConfig {
            id: "test-download-client".to_string(),
            name: "Test Download Client".to_string(),
            client_type: client_type.clone(),
            config_json,
            client_priority: 0,
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            created_at: now,
            updated_at: now,
        };
        let client = provider.client_for_config(&config).ok_or_else(|| {
            AppError::Validation(format!(
                "test connection is not supported for client type '{client_type}'"
            ))
        })?;
        client.test_connection().await?;
        Ok(())
    }

    async fn build_available_plugins(
        &self,
        actor: Option<&User>,
    ) -> AppResult<Vec<RegistryPlugin>> {
        let installations = self
            .services
            .customization
            .plugin_installations
            .list_plugin_installations()
            .await?;
        let install_in_progress_ids = match actor {
            Some(actor) => {
                self.runtime
                    .plugins
                    .plugin_install_orchestrator
                    .active_plugin_ids_for_actor(&actor.id)
                    .await
            }
            None => HashSet::new(),
        };

        let sources = self
            .services
            .customization
            .plugin_installations
            .list_plugin_catalog_sources()
            .await?;
        let central = sources
            .iter()
            .find(|source| source.source_key == CENTRAL_CATALOG_SOURCE_KEY)
            .and_then(|source| source.catalog_json.as_deref())
            .and_then(|json| parse_and_validate_central_catalog(json.as_bytes()).ok());
        let builtin_by_key = self.builtin_seed_by_key();
        let effective_installations = installations.iter().collect::<Vec<_>>();

        let mut result = Vec::new();

        if let Some(central) = central {
            for entry in central.plugins {
                let inst = effective_installations
                    .iter()
                    .copied()
                    .find(|installation| installation.plugin_id == entry.id);
                let Some(child_json) = sources
                    .iter()
                    .find(|source| source.source_key == child_catalog_source_key(&entry.id))
                    .and_then(|source| source.catalog_json.as_deref())
                else {
                    continue;
                };
                let Ok(child) =
                    parse_and_validate_child_catalog(child_json.as_bytes(), Some(&entry), None)
                else {
                    continue;
                };

                let plugin_type =
                    merged_plugin_type(&child.plugin_type, inst.map(|i| i.plugin_type.as_str()));
                if is_reserved_first_party_provider(&child.provider_type) {
                    continue;
                }
                let builtin = inst
                    .map(|installation| installation.is_builtin)
                    .unwrap_or_else(|| {
                        builtin_by_key
                            .contains_key(&builtin_lookup_key(&plugin_type, &child.provider_type))
                    });
                let selected_release = latest_compatible_child_release(&child);
                let latest_release = latest_child_release(&child);
                let blocked_release = latest_host_blocked_child_release(&child);
                let active_release =
                    inst.and_then(|installation| installed_child_release(&child, installation));
                let display_release = selected_release
                    .clone()
                    .or_else(|| active_release.clone())
                    .or_else(|| latest_release.clone());

                if inst.is_none() && display_release.is_none() && !builtin {
                    continue;
                }

                let version = display_release
                    .as_ref()
                    .map(|release| release.version.clone())
                    .or_else(|| inst.map(|installation| installation.version.clone()))
                    .unwrap_or_default();
                let latest_version = match (selected_release.as_ref(), latest_release.as_ref()) {
                    (Some(selected), Some(latest)) => {
                        let selected_version = parse_child_release_version(&entry.id, selected);
                        let latest_semver = parse_child_release_version(&entry.id, latest);
                        match selected_version.zip(latest_semver) {
                            Some((selected_version, latest_semver))
                                if latest_semver > selected_version =>
                            {
                                Some(latest.version.clone())
                            }
                            _ => None,
                        }
                    }
                    (None, Some(latest)) => Some(latest.version.clone()),
                    _ => None,
                };
                let blocked_reason = if selected_release.is_none() && latest_release.is_some() {
                    Some("no_compatible_release".to_string())
                } else if blocked_release.is_some() {
                    Some("newer_release_requires_newer_scryer".to_string())
                } else {
                    None
                };
                let update_available = inst
                    .and_then(|installation| {
                        selected_release.as_ref().and_then(|release| {
                            parse_child_release_version(&entry.id, release)
                                .zip(semver::Version::parse(installation.version.as_str()).ok())
                        })
                    })
                    .is_some_and(|(selected_version, installed_version)| {
                        selected_version > installed_version
                    });

                result.push(RegistryPlugin {
                    id: child.id.clone(),
                    name: child.name.clone(),
                    description: child.description.clone(),
                    version,
                    latest_version,
                    plugin_type: plugin_type.clone(),
                    provider_type: child.provider_type.clone(),
                    author: child.publisher.clone(),
                    official: entry.support_tier == PluginSupportTier::Official,
                    publisher: Some(child.publisher.clone()),
                    support_tier: entry.support_tier,
                    docs_url: Some(child.docs_url.clone()),
                    source_repo: Some(child.source_repo.clone()),
                    builtin,
                    source_url: inst
                        .and_then(|installation| installation.source_url.clone())
                        .or_else(|| {
                            display_release
                                .as_ref()
                                .map(|release| release.artifact_manifest_url.clone())
                        }),
                    source_kind: inst
                        .map(|installation| source_kind_label(installation.source_kind))
                        .or_else(|| Some(source_kind_label(PluginSourceKind::Downloaded))),
                    blocked_reason,
                    wasm_url: display_release
                        .as_ref()
                        .map(|release| release.artifact_manifest_url.clone()),
                    wasm_sha256: None,
                    min_scryer_version: None,
                    default_base_url: self
                        .default_base_url_for_plugin(&plugin_type, &child.provider_type),
                    is_installed: inst.is_some(),
                    is_enabled: inst.map(|i| i.is_enabled).unwrap_or(false),
                    installed_version: inst.map(|i| i.version.clone()),
                    update_available,
                    install_in_progress: install_in_progress_ids.contains(&child.id),
                });
            }
        }

        for resolved in self
            .resolved_catalog_plugins()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|resolved| resolved.central.is_none())
        {
            let inst = effective_installations
                .iter()
                .copied()
                .find(|installation| installation.plugin_id == resolved.child.id);
            if is_reserved_first_party_provider(&resolved.child.provider_type) {
                continue;
            }
            let plugin_type = merged_plugin_type(
                &resolved.child.plugin_type,
                inst.map(|i| i.plugin_type.as_str()),
            );
            let builtin = inst
                .map(|installation| installation.is_builtin)
                .unwrap_or_else(|| {
                    builtin_by_key.contains_key(&builtin_lookup_key(
                        &plugin_type,
                        &resolved.child.provider_type,
                    ))
                });
            let update_available = inst
                .and_then(|installation| {
                    semver::Version::parse(resolved.release.version.trim_start_matches('v'))
                        .ok()
                        .zip(semver::Version::parse(installation.version.as_str()).ok())
                })
                .is_some_and(|(selected_version, installed_version)| {
                    selected_version > installed_version
                });

            result.push(RegistryPlugin {
                id: resolved.child.id.clone(),
                name: resolved.child.name.clone(),
                description: resolved.child.description.clone(),
                version: resolved.release.version.clone(),
                latest_version: None,
                plugin_type: plugin_type.clone(),
                provider_type: resolved.child.provider_type.clone(),
                author: resolved.child.publisher.clone(),
                official: resolved.effective_support_tier == PluginSupportTier::Official,
                publisher: Some(resolved.child.publisher.clone()),
                support_tier: resolved.effective_support_tier,
                docs_url: Some(resolved.child.docs_url.clone()),
                source_repo: Some(resolved.child.source_repo.clone()),
                builtin,
                source_url: inst
                    .and_then(|installation| installation.source_url.clone())
                    .or_else(|| Some(resolved.release.artifact_manifest_url.clone())),
                source_kind: inst
                    .map(|installation| source_kind_label(installation.source_kind))
                    .or_else(|| Some(source_kind_label(resolved.source_kind))),
                blocked_reason: None,
                wasm_url: Some(resolved.release.artifact_manifest_url.clone()),
                wasm_sha256: None,
                min_scryer_version: None,
                default_base_url: self
                    .default_base_url_for_plugin(&plugin_type, &resolved.child.provider_type),
                is_installed: inst.is_some(),
                is_enabled: inst.map(|i| i.is_enabled).unwrap_or(false),
                installed_version: inst.map(|i| i.version.clone()),
                update_available,
                install_in_progress: install_in_progress_ids.contains(&resolved.child.id),
            });
        }

        for inst in effective_installations {
            if is_reserved_first_party_provider(&inst.provider_type) {
                continue;
            }
            if !result.iter().any(|r| r.id == inst.plugin_id) {
                let builtin = builtin_by_key
                    .contains_key(&builtin_lookup_key(&inst.plugin_type, &inst.provider_type))
                    || inst.is_builtin;
                result.push(RegistryPlugin {
                    id: inst.plugin_id.clone(),
                    name: inst.name.clone(),
                    description: inst.description.clone(),
                    version: inst.version.clone(),
                    latest_version: None,
                    plugin_type: inst.plugin_type.clone(),
                    provider_type: inst.provider_type.clone(),
                    author: String::new(),
                    official: false,
                    publisher: inst.publisher.clone(),
                    support_tier: inst.support_tier,
                    docs_url: inst.docs_url.clone(),
                    source_repo: inst.source_repo.clone(),
                    builtin,
                    source_url: inst.source_url.clone(),
                    source_kind: Some(source_kind_label(inst.source_kind)),
                    blocked_reason: None,
                    wasm_url: None,
                    wasm_sha256: None,
                    min_scryer_version: None,
                    default_base_url: self
                        .default_base_url_for_plugin(&inst.plugin_type, &inst.provider_type),
                    is_installed: true,
                    is_enabled: inst.is_enabled,
                    installed_version: Some(inst.version.clone()),
                    update_available: false,
                    install_in_progress: install_in_progress_ids.contains(&inst.plugin_id),
                });
            }
        }

        Ok(result)
    }

    /// List available plugins by merging cached registry with local installations.
    pub async fn list_available_plugins(&self, actor: &User) -> AppResult<Vec<RegistryPlugin>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        self.build_available_plugins(Some(actor)).await
    }

    pub async fn plugin_update_count(&self, actor: &User) -> AppResult<i64> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        Ok(self
            .build_available_plugins(None)
            .await?
            .into_iter()
            .filter(|plugin| plugin.update_available)
            .count() as i64)
    }

    async fn cached_central_catalog(&self) -> AppResult<Option<CentralCatalog>> {
        let Some(source) = self
            .services
            .customization
            .plugin_installations
            .get_plugin_catalog_source(CENTRAL_CATALOG_SOURCE_KEY)
            .await?
        else {
            return Ok(None);
        };
        let Some(json) = source.catalog_json else {
            return Ok(None);
        };
        match parse_and_validate_central_catalog(json.as_bytes()) {
            Ok(catalog) => Ok(Some(catalog)),
            Err(error) => {
                warn!(error = %error, "cached central plugin catalog is invalid");
                Ok(None)
            }
        }
    }

    async fn load_rule_pack_catalog(&self) -> AppResult<CentralCatalog> {
        if let Some(catalog) = self.cached_central_catalog().await? {
            return Ok(catalog);
        }

        self.refresh_plugin_catalog_internal().await?;
        self.cached_central_catalog().await?.ok_or_else(|| {
            AppError::Repository("central plugin catalog is unavailable".to_string())
        })
    }

    /// Refresh the plugin registry from the remote URL.
    pub async fn refresh_plugin_registry(&self, actor: &User) -> AppResult<Vec<RegistryPlugin>> {
        self.refresh_plugin_catalog(actor).await
    }

    /// Internal registry refresh (no auth check) for use by startup and background tasks.
    pub async fn refresh_plugin_registry_internal(&self) -> AppResult<()> {
        self.refresh_plugin_catalog_internal().await
    }

    pub async fn plugin_catalog_status(&self, actor: &User) -> AppResult<PluginCatalogStatus> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let now = Utc::now();
        let status = match fetch_plugin_bytes(
            GITHUB_STATUS_SUMMARY_URL,
            "GitHub Status",
            "github_status",
        )
        .await
        {
            Ok(raw) => github_outage_status_from_summary(&raw).unwrap_or_else(|| {
                // Fail open if GitHub Status is malformed or missing fields we understand.
                super::catalog::CatalogOutageStatus {
                    github_available: true,
                    blocked_actions: Vec::new(),
                    message: None,
                }
            }),
            Err(error) => {
                debug!(error = %error, "GitHub Status unavailable; failing plugin catalog gating open");
                super::catalog::CatalogOutageStatus {
                    github_available: true,
                    blocked_actions: Vec::new(),
                    message: None,
                }
            }
        };

        let payload = serde_json::json!({
            "githubAvailable": status.github_available,
            "blockedActions": status.blocked_actions,
            "message": status.message,
        });
        self.services
            .customization
            .plugin_installations
            .upsert_plugin_catalog_status(&PluginCatalogStatusRecord {
                status_key: CATALOG_STATUS_KEY.to_string(),
                status_json: payload.to_string(),
                checked_at: now,
            })
            .await?;

        let last_error = self
            .services
            .customization
            .plugin_installations
            .list_plugin_catalog_sources()
            .await?
            .into_iter()
            .find_map(|source| source.last_error);

        Ok(PluginCatalogStatus {
            refresh_state: if last_error.is_some() {
                "degraded".to_string()
            } else {
                "ready".to_string()
            },
            github_available: status.github_available,
            last_checked_at: Some(now.to_rfc3339()),
            outage_message: status.message,
            blocked_actions: status.blocked_actions,
            last_error,
        })
    }

    pub async fn refresh_plugin_catalog(&self, actor: &User) -> AppResult<Vec<RegistryPlugin>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.refresh_plugin_catalog_internal().await?;
        self.list_available_plugins(actor).await
    }

    pub async fn refresh_plugin_catalog_internal(&self) -> AppResult<()> {
        let catalog_url = plugin_catalog_url();
        let central_raw = self
            .fetch_verified_catalog_bytes(
                &catalog_url,
                &RequiredSigner {
                    github_repository: CENTRAL_CATALOG_REPO.to_string(),
                    github_workflow: Some(CENTRAL_CATALOG_WORKFLOW.to_string()),
                },
                "central plugin catalog",
            )
            .await?;
        let central = parse_and_validate_central_catalog(&central_raw)?;
        let central_json = String::from_utf8(central_raw.clone()).map_err(|e| {
            AppError::Validation(format!("central plugin catalog is not UTF-8: {e}"))
        })?;
        let now = Utc::now();
        self.services
            .customization
            .plugin_installations
            .upsert_plugin_catalog_source(&PluginCatalogSource {
                source_key: CENTRAL_CATALOG_SOURCE_KEY.to_string(),
                source_kind: "central".to_string(),
                source_url: catalog_url,
                github_repo: Some(CENTRAL_CATALOG_REPO.to_string()),
                support_tier: PluginSupportTier::Official,
                catalog_json: Some(central_json),
                last_success_at: Some(now),
                last_error: None,
                updated_at: now,
            })
            .await?;
        let mut remaining_entries = central.plugins.into_iter();
        let mut refresh_tasks = tokio::task::JoinSet::new();
        for _ in 0..CHILD_CATALOG_REFRESH_CONCURRENCY {
            let Some(entry) = remaining_entries.next() else {
                break;
            };
            let app = self.clone();
            let refreshed_at = now;
            refresh_tasks.spawn(async move {
                app.refresh_child_catalog_source_entry(entry, refreshed_at)
                    .await
            });
        }

        while let Some(result) = refresh_tasks.join_next().await {
            let outcome = result.map_err(|error| {
                AppError::Repository(format!(
                    "child plugin catalog refresh task panicked: {error}"
                ))
            })??;
            self.services
                .customization
                .plugin_installations
                .upsert_plugin_catalog_source(&outcome.source)
                .await?;

            if let Some(entry) = remaining_entries.next() {
                let app = self.clone();
                let refreshed_at = now;
                refresh_tasks.spawn(async move {
                    app.refresh_child_catalog_source_entry(entry, refreshed_at)
                        .await
                });
            }
        }

        for source in self
            .services
            .customization
            .plugin_installations
            .list_plugin_catalog_sources()
            .await?
            .into_iter()
            .filter(|source| source.source_kind == "manual")
        {
            let source_url = source.source_url.clone();
            let result = async {
                let repo_slug = source.github_repo.as_deref().ok_or_else(|| {
                    AppError::Validation(format!(
                        "manual plugin catalog source '{}' is missing github repo",
                        source.source_key
                    ))
                })?;
                let repo = GitHubRepo::parse(repo_slug)?;
                let child_url = if source_url.trim().is_empty() {
                    repo.child_catalog_url()
                } else {
                    source_url.clone()
                };
                let (_, child_json) = self
                    .resolve_manual_plugin_repo_at_url(repo.clone(), &child_url)
                    .await?;
                self.upsert_manual_plugin_catalog_source(&repo, &child_url, Some(child_json), None)
                    .await
            }
            .await;

            if let Err(error) = result {
                warn!(
                    source_key = source.source_key.as_str(),
                    error = %error,
                    "verified manual plugin catalog is unavailable"
                );
                if let Some(repo) = source
                    .github_repo
                    .as_deref()
                    .and_then(|repo| GitHubRepo::parse(repo).ok())
                {
                    let child_url = if source_url.trim().is_empty() {
                        repo.child_catalog_url()
                    } else {
                        source_url.clone()
                    };
                    self.upsert_manual_plugin_catalog_source(
                        &repo,
                        &child_url,
                        None,
                        Some(error.to_string()),
                    )
                    .await?;
                }
            }
        }

        self.publish_provider_catalog_changed(ProviderCatalogFamily::all().into_iter().collect());
        Ok(())
    }

    async fn refresh_child_catalog_source_entry(
        &self,
        entry: CentralCatalogEntry,
        refreshed_at: chrono::DateTime<Utc>,
    ) -> AppResult<ChildCatalogRefreshOutcome> {
        let source_key = child_catalog_source_key(&entry.id);
        let source_repo = GitHubRepo::parse(&entry.source_repo)?;
        let result = async {
            let child_raw = self
                .fetch_verified_catalog_bytes(
                    &entry.child_catalog_url,
                    &entry.required_signer,
                    "child plugin catalog",
                )
                .await?;
            parse_and_validate_child_catalog(&child_raw, Some(&entry), None)?;
            String::from_utf8(child_raw).map_err(|e| {
                AppError::Validation(format!("child plugin catalog is not UTF-8: {e}"))
            })
        }
        .await;

        let source = match result {
            Ok(child_json) => PluginCatalogSource {
                source_key,
                source_kind: "child".to_string(),
                source_url: entry.child_catalog_url,
                github_repo: Some(source_repo.slug()),
                support_tier: entry.support_tier,
                catalog_json: Some(child_json),
                last_success_at: Some(refreshed_at),
                last_error: None,
                updated_at: refreshed_at,
            },
            Err(error) => {
                warn!(
                    plugin_id = entry.id.as_str(),
                    error = %error,
                    "verified child plugin catalog is unavailable"
                );
                PluginCatalogSource {
                    source_key,
                    source_kind: "child".to_string(),
                    source_url: entry.child_catalog_url,
                    github_repo: Some(source_repo.slug()),
                    support_tier: entry.support_tier,
                    catalog_json: None,
                    last_success_at: None,
                    last_error: Some(error.to_string()),
                    updated_at: refreshed_at,
                }
            }
        };

        Ok(ChildCatalogRefreshOutcome { source })
    }

    async fn fetch_verified_catalog_bytes(
        &self,
        url: &str,
        signer: &RequiredSigner,
        label: &str,
    ) -> AppResult<Vec<u8>> {
        let scope = format!("catalog_blob:{}", blake3_digest(url.as_bytes()));
        let raw = fetch_plugin_bytes(url, label, scope.clone()).await?;
        let bundle = fetch_plugin_bytes(
            &bundle_url_for(url),
            &format!("{label} bundle"),
            format!("{scope}:bundle"),
        )
        .await?;
        verify_signed_blob(raw.clone(), bundle, signer.clone()).await?;
        if url.ends_with(".zst") {
            decompress_zstd(raw).await
        } else {
            Ok(raw)
        }
    }

    async fn resolved_catalog_plugins(&self) -> AppResult<Vec<CatalogPluginResolution>> {
        let sources = self
            .services
            .customization
            .plugin_installations
            .list_plugin_catalog_sources()
            .await?;
        let central = sources
            .iter()
            .find(|source| source.source_key == CENTRAL_CATALOG_SOURCE_KEY)
            .and_then(|source| source.catalog_json.as_deref())
            .and_then(|json| parse_and_validate_central_catalog(json.as_bytes()).ok());

        let mut result = Vec::new();
        if let Some(central) = central {
            for entry in central.plugins {
                let Some(child_json) = sources
                    .iter()
                    .find(|source| source.source_key == child_catalog_source_key(&entry.id))
                    .and_then(|source| source.catalog_json.as_deref())
                else {
                    continue;
                };
                let child =
                    parse_and_validate_child_catalog(child_json.as_bytes(), Some(&entry), None)?;
                let Some(release) = latest_compatible_child_release(&child) else {
                    continue;
                };
                let github_repo = GitHubRepo::parse(&child.source_repo)?;
                result.push(CatalogPluginResolution {
                    effective_support_tier: entry.support_tier,
                    central: Some(entry),
                    child,
                    release,
                    source_kind: PluginSourceKind::Downloaded,
                    github_repo,
                });
            }
        }

        for source in sources
            .iter()
            .filter(|source| source.source_kind == "manual")
            .filter_map(|source| {
                source
                    .catalog_json
                    .as_deref()
                    .zip(source.github_repo.as_deref())
            })
        {
            let (child_json, repo_slug) = source;
            let manual_repo = GitHubRepo::parse(repo_slug)?;
            let child =
                parse_and_validate_child_catalog(child_json.as_bytes(), None, Some(&manual_repo))?;
            let Some(release) = latest_compatible_child_release(&child) else {
                continue;
            };
            result.push(CatalogPluginResolution {
                central: None,
                child,
                release,
                source_kind: PluginSourceKind::Manual,
                effective_support_tier: PluginSupportTier::Unverified,
                github_repo: manual_repo,
            });
        }

        Ok(result)
    }

    async fn fetch_catalog_release_wasm(
        &self,
        resolved: &CatalogPluginResolution,
        reporter: &PluginInstallProgressReporter,
    ) -> AppResult<(Vec<u8>, Vec<u8>, PluginReleaseManifest, String)> {
        reporter.downloading().await;
        let signer = resolved
            .central
            .as_ref()
            .map(|central| central.required_signer.clone())
            .unwrap_or_else(|| RequiredSigner {
                github_repository: resolved.github_repo.slug(),
                github_workflow: None,
            });
        let manifest_raw = self
            .fetch_verified_catalog_bytes(
                &resolved.release.artifact_manifest_url,
                &signer,
                "plugin release manifest",
            )
            .await?;
        let manifest = parse_and_validate_release_manifest(
            &manifest_raw,
            &resolved.child,
            &resolved.release,
            &resolved.github_repo,
        )?;
        let artifact_url =
            plugin_manifest_asset_url(&resolved.release.artifact_manifest_url, &manifest.artifact)?;
        let artifact_bundle_url = plugin_manifest_asset_url(
            &resolved.release.artifact_manifest_url,
            &manifest.signature,
        )?;
        let compressed_artifact = fetch_plugin_bytes(
            &artifact_url,
            "plugin artifact",
            format!("plugin_release_asset:{}", resolved.child.id),
        )
        .await?;
        let artifact_bundle = fetch_plugin_bytes(
            &artifact_bundle_url,
            "plugin artifact bundle",
            format!("plugin_release_asset:{}:bundle", resolved.child.id),
        )
        .await?;
        reporter.verifying().await;
        verify_signed_blob(compressed_artifact.clone(), artifact_bundle, signer).await?;
        verify_digest(
            "compressed plugin artifact",
            &manifest.artifact_digest,
            &compressed_artifact,
        )?;
        let wasm = decompress_zstd(compressed_artifact.clone()).await?;
        verify_digest("decompressed plugin WASM", &manifest.wasm_digest, &wasm)?;
        let persisted_wasm = compress_zstd(wasm.clone(), SQLITE_PLUGIN_WASM_ZSTD_LEVEL).await?;
        Ok((persisted_wasm, wasm, manifest, artifact_url))
    }

    async fn install_catalog_plugin(
        &self,
        resolved: CatalogPluginResolution,
        reporter: &PluginInstallProgressReporter,
    ) -> AppResult<PluginInstallation> {
        if is_reserved_first_party_provider(&resolved.child.provider_type) {
            return Err(AppError::Validation(format!(
                "provider type '{}' is reserved for first-party code",
                resolved.child.provider_type
            )));
        }

        let (compressed_wasm_bytes, wasm_bytes, manifest, artifact_url) =
            self.fetch_catalog_release_wasm(&resolved, reporter).await?;
        let release = DownloadedPluginReleaseContract {
            version: resolved.release.version.clone(),
            sdk_version: None,
            sdk_constraint: resolved.release.sdk_constraint.clone(),
            scryer_constraint: None,
        };
        let validated = validate_catalog_downloaded_plugin_release(
            &resolved.child.id,
            &resolved.child.plugin_type,
            &resolved.child.provider_type,
            &release,
            &wasm_bytes,
        )?;

        let (wasm_digest_algo, wasm_digest) = parse_digest_string(&manifest.wasm_digest)?;
        let now = Utc::now();
        let installation = PluginInstallation {
            id: Id::new().0,
            plugin_id: resolved.child.id.clone(),
            name: validated.descriptor.name.clone(),
            description: resolved.child.description.clone(),
            version: validated.descriptor.version.clone(),
            sdk_version: validated.descriptor.sdk_version.clone(),
            sdk_constraint: validated.sdk_constraint.clone(),
            scryer_constraint: None,
            plugin_type: validated.descriptor.plugin_type().to_string(),
            provider_type: normalize_provider_key(validated.descriptor.provider_type()),
            source_kind: resolved.source_kind,
            is_enabled: true,
            is_builtin: false,
            wasm_encoding: PluginWasmEncoding::Zstd,
            wasm_digest_algo: Some(wasm_digest_algo),
            source_url: Some(artifact_url),
            support_tier: resolved.effective_support_tier,
            publisher: Some(resolved.child.publisher.clone()),
            docs_url: Some(resolved.child.docs_url.clone()),
            source_repo: Some(resolved.child.source_repo.clone()),
            manifest_url: Some(resolved.release.artifact_manifest_url.clone()),
            wasm_digest: Some(wasm_digest),
            artifact_digest: Some(manifest.artifact_digest),
            descriptor_json: Some(persisted_plugin_descriptor_json(&validated.descriptor)?),
            installed_at: now,
            updated_at: now,
        };

        reporter.installing().await;
        let result = self
            .services
            .customization
            .plugin_installations
            .create_plugin_installation(&installation, Some(compressed_wasm_bytes.as_slice()))
            .await?;

        self.apply_runtime_plugin_upsert(
            &result,
            runtime_plugin_load_from_validated(
                validated.descriptor,
                wasm_bytes,
                catalog_resolution_is_first_party(&resolved),
            ),
        )?;
        self.finalize_runtime_plugin_mutation(&result.plugin_type, true)
            .await?;
        Ok(result)
    }

    async fn upgrade_catalog_plugin(
        &self,
        resolved: CatalogPluginResolution,
        installation: PluginInstallation,
        reporter: &PluginInstallProgressReporter,
    ) -> AppResult<PluginInstallation> {
        let selected_version = semver::Version::parse(
            resolved.release.version.trim_start_matches('v'),
        )
        .map_err(|e| {
            AppError::Validation(format!(
                "invalid catalog version '{}': {e}",
                resolved.release.version
            ))
        })?;
        let installed_version = semver::Version::parse(&installation.version).map_err(|e| {
            AppError::Validation(format!(
                "invalid installed version '{}': {e}",
                installation.version
            ))
        })?;
        if selected_version <= installed_version {
            return Err(AppError::Validation(format!(
                "plugin '{}' is already at version {} (selected release is {})",
                resolved.child.id, installation.version, resolved.release.version
            )));
        }

        let (compressed_wasm_bytes, wasm_bytes, manifest, artifact_url) =
            self.fetch_catalog_release_wasm(&resolved, reporter).await?;
        let release = DownloadedPluginReleaseContract {
            version: resolved.release.version.clone(),
            sdk_version: None,
            sdk_constraint: resolved.release.sdk_constraint.clone(),
            scryer_constraint: None,
        };
        let validated = validate_catalog_downloaded_plugin_release(
            &resolved.child.id,
            &resolved.child.plugin_type,
            &resolved.child.provider_type,
            &release,
            &wasm_bytes,
        )?;

        let (wasm_digest_algo, wasm_digest) = parse_digest_string(&manifest.wasm_digest)?;
        let previous_plugin_type = installation.plugin_type.clone();
        let previous_provider_type = installation.provider_type.clone();
        let mut updated = installation;
        updated.version = validated.descriptor.version.clone();
        updated.name = validated.descriptor.name.clone();
        updated.description = resolved.child.description.clone();
        updated.sdk_version = validated.descriptor.sdk_version.clone();
        updated.sdk_constraint = validated.sdk_constraint.clone();
        updated.scryer_constraint = None;
        updated.plugin_type = validated.descriptor.plugin_type().to_string();
        updated.provider_type = normalize_provider_key(validated.descriptor.provider_type());
        updated.source_kind = resolved.source_kind;
        updated.wasm_encoding = PluginWasmEncoding::Zstd;
        updated.wasm_digest_algo = Some(wasm_digest_algo);
        updated.source_url = Some(artifact_url);
        updated.support_tier = resolved.effective_support_tier;
        updated.publisher = Some(resolved.child.publisher.clone());
        updated.docs_url = Some(resolved.child.docs_url.clone());
        updated.source_repo = Some(resolved.child.source_repo.clone());
        updated.manifest_url = Some(resolved.release.artifact_manifest_url.clone());
        updated.wasm_digest = Some(wasm_digest);
        updated.artifact_digest = Some(manifest.artifact_digest);
        updated.descriptor_json = Some(persisted_plugin_descriptor_json(&validated.descriptor)?);
        updated.updated_at = Utc::now();

        reporter.installing().await;
        let result = self
            .services
            .customization
            .plugin_installations
            .update_plugin_installation(&updated, Some(compressed_wasm_bytes.as_slice()))
            .await?;

        let runtime_touched = result.is_enabled;
        if runtime_touched {
            let mut previous_runtime_installation = result.clone();
            previous_runtime_installation.plugin_type = previous_plugin_type.clone();
            previous_runtime_installation.provider_type = previous_provider_type.clone();
            self.apply_runtime_plugin_replace(
                &previous_runtime_installation,
                &result,
                runtime_plugin_load_from_validated(
                    validated.descriptor,
                    wasm_bytes,
                    catalog_resolution_is_first_party(&resolved),
                ),
            )?;
        }
        self.finalize_runtime_plugin_mutation_for_types(
            [previous_plugin_type.as_str(), result.plugin_type.as_str()],
            runtime_touched,
        )
        .await?;
        Ok(result)
    }

    async fn upsert_manual_plugin_catalog_source(
        &self,
        repo: &GitHubRepo,
        source_url: &str,
        child_json: Option<String>,
        last_error: Option<String>,
    ) -> AppResult<()> {
        let now = Utc::now();
        let last_success_at = child_json.as_ref().map(|_| now);
        self.services
            .customization
            .plugin_installations
            .upsert_plugin_catalog_source(&PluginCatalogSource {
                source_key: manual_catalog_source_key(repo),
                source_kind: "manual".to_string(),
                source_url: source_url.to_string(),
                github_repo: Some(repo.slug()),
                support_tier: PluginSupportTier::Unverified,
                catalog_json: child_json,
                last_success_at,
                last_error,
                updated_at: now,
            })
            .await
    }

    async fn resolve_manual_plugin_repo(
        &self,
        github_repo_url: &str,
    ) -> AppResult<(CatalogPluginResolution, String)> {
        let repo = GitHubRepo::parse(github_repo_url)?;
        let child_url = repo.child_catalog_url();
        self.resolve_manual_plugin_repo_at_url(repo, &child_url)
            .await
    }

    async fn resolve_manual_plugin_repo_at_url(
        &self,
        repo: GitHubRepo,
        child_url: &str,
    ) -> AppResult<(CatalogPluginResolution, String)> {
        let signer = RequiredSigner {
            github_repository: repo.slug(),
            github_workflow: None,
        };
        let child_raw = self
            .fetch_verified_catalog_bytes(child_url, &signer, "manual plugin child catalog")
            .await?;
        let child = parse_and_validate_child_catalog(&child_raw, None, Some(&repo))?;
        let release = latest_compatible_child_release(&child).ok_or_else(|| {
            AppError::Validation(format!(
                "manual plugin repo '{}' has no SDK-compatible release",
                repo.slug()
            ))
        })?;
        let child_json = String::from_utf8(child_raw).map_err(|e| {
            AppError::Validation(format!("manual child plugin catalog is not UTF-8: {e}"))
        })?;
        Ok((
            CatalogPluginResolution {
                central: None,
                child,
                release,
                source_kind: PluginSourceKind::Manual,
                effective_support_tier: PluginSupportTier::Unverified,
                github_repo: repo,
            },
            child_json,
        ))
    }

    pub async fn inspect_manual_plugin_repo(
        &self,
        actor: &User,
        github_repo_url: &str,
    ) -> AppResult<ManualPluginPreview> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let (resolved, _) = self.resolve_manual_plugin_repo(github_repo_url).await?;
        let plugin_type = resolved.child.plugin_type.clone();
        Ok(ManualPluginPreview {
            github_repo_url: format!("https://github.com/{}", resolved.github_repo.slug()),
            plugin: RegistryPlugin {
                id: resolved.child.id.clone(),
                name: resolved.child.name.clone(),
                description: resolved.child.description.clone(),
                version: resolved.release.version.clone(),
                latest_version: None,
                plugin_type: plugin_type.clone(),
                provider_type: resolved.child.provider_type.clone(),
                author: resolved.child.publisher.clone(),
                official: false,
                publisher: Some(resolved.child.publisher.clone()),
                support_tier: PluginSupportTier::Unverified,
                docs_url: Some(resolved.child.docs_url.clone()),
                source_repo: Some(resolved.child.source_repo.clone()),
                builtin: false,
                source_url: Some(resolved.release.artifact_manifest_url.clone()),
                source_kind: Some(source_kind_label(PluginSourceKind::Manual)),
                blocked_reason: None,
                wasm_url: Some(resolved.release.artifact_manifest_url.clone()),
                wasm_sha256: None,
                min_scryer_version: None,
                is_installed: self
                    .services
                    .customization
                    .plugin_installations
                    .get_plugin_installation(&resolved.child.id)
                    .await?
                    .is_some(),
                is_enabled: false,
                installed_version: None,
                update_available: false,
                install_in_progress: false,
                default_base_url: self
                    .default_base_url_for_plugin(&plugin_type, &resolved.child.provider_type),
            },
        })
    }

    pub async fn install_manual_plugin(
        &self,
        actor: &User,
        github_repo_url: &str,
    ) -> AppResult<PluginInstallation> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let (resolved, child_json) = self.resolve_manual_plugin_repo(github_repo_url).await?;
        if is_reserved_first_party_provider(&resolved.child.provider_type) {
            return Err(AppError::Validation(format!(
                "provider type '{}' is reserved for first-party code",
                resolved.child.provider_type
            )));
        }
        let _operation_guard = self
            .runtime
            .plugins
            .plugin_operation_guards
            .acquire(&resolved.child.id)
            .await;
        if self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(&resolved.child.id)
            .await?
            .is_some()
        {
            return Err(AppError::Validation(format!(
                "plugin '{}' is already installed",
                resolved.child.id
            )));
        }
        let child_url = resolved.github_repo.child_catalog_url();
        self.upsert_manual_plugin_catalog_source(
            &resolved.github_repo,
            &child_url,
            Some(child_json),
            None,
        )
        .await?;
        let reporter = PluginInstallProgressReporter::new(self, &actor.id, &resolved.child.id);
        self.install_catalog_plugin(resolved, &reporter).await
    }

    async fn validate_catalog_install_request(&self, plugin_id: &str) -> AppResult<()> {
        if self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .is_some()
        {
            return Err(AppError::Validation(format!(
                "plugin '{plugin_id}' is already installed"
            )));
        }

        let available = self
            .resolved_catalog_plugins()
            .await?
            .into_iter()
            .any(|plugin| plugin.child.id == plugin_id);
        if available {
            Ok(())
        } else {
            Err(AppError::NotFound(format!(
                "plugin '{plugin_id}' is not available from the plugin catalog"
            )))
        }
    }

    async fn validate_catalog_upgrade_request(&self, plugin_id: &str) -> AppResult<()> {
        self.services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not installed")))?;
        Ok(())
    }

    fn plugin_install_in_progress_error(plugin_id: &str) -> AppError {
        AppError::PluginInstallInProgress(plugin_id.trim().to_ascii_lowercase())
    }

    async fn perform_catalog_install(
        &self,
        plugin_id: &str,
        reporter: &PluginInstallProgressReporter,
    ) -> AppResult<PluginInstallation> {
        if self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .is_some()
        {
            return Err(AppError::Validation(format!(
                "plugin '{plugin_id}' is already installed"
            )));
        }

        if let Some(resolved) = self
            .resolved_catalog_plugins()
            .await?
            .into_iter()
            .find(|plugin| plugin.child.id == plugin_id)
        {
            return self.install_catalog_plugin(resolved, reporter).await;
        }
        Err(AppError::NotFound(format!(
            "plugin '{plugin_id}' is not available from the plugin catalog"
        )))
    }

    async fn perform_catalog_upgrade(
        &self,
        plugin_id: &str,
        reporter: &PluginInstallProgressReporter,
    ) -> AppResult<PluginInstallation> {
        let installation = self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not installed")))?;

        if installation.source_kind == PluginSourceKind::Manual {
            let source_repo = installation.source_repo.as_deref().ok_or_else(|| {
                AppError::Validation(format!(
                    "manual plugin '{plugin_id}' is missing source repo"
                ))
            })?;
            let (resolved, child_json) = self.resolve_manual_plugin_repo(source_repo).await?;
            let child_url = resolved.github_repo.child_catalog_url();
            self.upsert_manual_plugin_catalog_source(
                &resolved.github_repo,
                &child_url,
                Some(child_json),
                None,
            )
            .await?;
            return self
                .upgrade_catalog_plugin(resolved, installation, reporter)
                .await;
        }

        if let Some(resolved) = self
            .resolved_catalog_plugins()
            .await?
            .into_iter()
            .find(|plugin| plugin.child.id == plugin_id)
        {
            return self
                .upgrade_catalog_plugin(resolved, installation, reporter)
                .await;
        }
        Err(AppError::NotFound(format!(
            "plugin '{plugin_id}' is not available from the plugin catalog"
        )))
    }

    pub async fn begin_install_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<PluginInstallProgressSnapshot> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.validate_catalog_install_request(plugin_id).await?;
        let snapshot = self
            .runtime
            .plugins
            .plugin_install_orchestrator
            .begin(&actor.id, plugin_id, PluginInstallOperationKind::Install)
            .await
            .map_err(|_| Self::plugin_install_in_progress_error(plugin_id))?;
        let app = self.clone();
        let actor = actor.clone();
        let plugin_id = plugin_id.trim().to_string();
        tokio::spawn(async move {
            let reporter = PluginInstallProgressReporter::new(&app, &actor.id, &plugin_id);
            let result = app.perform_catalog_install(&plugin_id, &reporter).await;
            match result {
                Ok(_) => reporter.succeeded().await,
                Err(error) => {
                    reporter.failed(&error).await;
                    tracing::warn!(
                        plugin_id = plugin_id.as_str(),
                        error = %error,
                        "plugin install operation failed"
                    );
                }
            }
        });
        Ok(snapshot)
    }

    pub async fn begin_upgrade_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<PluginInstallProgressSnapshot> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.validate_catalog_upgrade_request(plugin_id).await?;
        let snapshot = self
            .runtime
            .plugins
            .plugin_install_orchestrator
            .begin(&actor.id, plugin_id, PluginInstallOperationKind::Upgrade)
            .await
            .map_err(|_| Self::plugin_install_in_progress_error(plugin_id))?;
        let app = self.clone();
        let actor = actor.clone();
        let plugin_id = plugin_id.trim().to_string();
        tokio::spawn(async move {
            let reporter = PluginInstallProgressReporter::new(&app, &actor.id, &plugin_id);
            let result = app.perform_catalog_upgrade(&plugin_id, &reporter).await;
            match result {
                Ok(_) => reporter.succeeded().await,
                Err(error) => {
                    reporter.failed(&error).await;
                    tracing::warn!(
                        plugin_id = plugin_id.as_str(),
                        error = %error,
                        "plugin upgrade operation failed"
                    );
                }
            }
        });
        Ok(snapshot)
    }

    /// Install a plugin from the registry.
    pub async fn install_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<PluginInstallation> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.validate_catalog_install_request(plugin_id).await?;
        self.runtime
            .plugins
            .plugin_install_orchestrator
            .begin(&actor.id, plugin_id, PluginInstallOperationKind::Install)
            .await
            .map_err(|_| Self::plugin_install_in_progress_error(plugin_id))?;
        let reporter = PluginInstallProgressReporter::new(self, &actor.id, plugin_id);
        let result = self.perform_catalog_install(plugin_id, &reporter).await;
        match &result {
            Ok(_) => reporter.succeeded().await,
            Err(error) => reporter.failed(error).await,
        }
        result
    }

    /// Uninstall a non-builtin plugin or revert a downloaded builtin override.
    pub async fn uninstall_plugin(&self, actor: &User, plugin_id: &str) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let installation = self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not installed")))?;

        if installation.is_builtin && installation.source_kind == PluginSourceKind::Bundled {
            return Err(AppError::Validation(
                "cannot uninstall built-in plugins; disable them instead".to_string(),
            ));
        }

        if installation.is_builtin && installation.source_kind == PluginSourceKind::Downloaded {
            let mut builtin_by_key = self.builtin_seed_by_key();
            let builtin_seed = builtin_by_key
                .remove(&builtin_lookup_key(
                    &installation.plugin_type,
                    &installation.provider_type,
                ))
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "cannot revert built-in plugin '{}' because no bundled definition is available",
                        plugin_id
                    ))
                })?;
            let mut reverted = installation.clone();
            reverted.name = builtin_seed.name;
            reverted.version = builtin_seed.version;
            reverted.sdk_version = builtin_seed.sdk_version;
            reverted.sdk_constraint = builtin_seed.sdk_constraint;
            reverted.scryer_constraint = None;
            reverted.plugin_type = builtin_seed.plugin_type;
            reverted.provider_type = builtin_seed.provider_type;
            reverted.source_kind = PluginSourceKind::Bundled;
            reverted.wasm_encoding = PluginWasmEncoding::Identity;
            reverted.wasm_digest_algo = None;
            reverted.source_url = None;
            reverted.manifest_url = None;
            reverted.wasm_digest = None;
            reverted.artifact_digest = None;
            reverted.updated_at = Utc::now();

            self.services
                .customization
                .plugin_installations
                .update_plugin_installation(&reverted, None)
                .await?;

            let runtime_touched = reverted.is_enabled;
            if runtime_touched {
                self.apply_runtime_builtin_restore(&reverted)?;
            } else {
                self.apply_runtime_plugin_removal(&reverted)?;
            }
            self.finalize_runtime_plugin_mutation(&reverted.plugin_type, runtime_touched)
                .await?;
            return Ok(());
        }

        // Delete all associated IndexerConfigs for this plugin's provider type.
        if is_indexer_plugin_type(&installation.plugin_type) {
            let configs = self
                .services
                .integrations
                .indexer_configs
                .list(Some(installation.provider_type.clone()))
                .await
                .unwrap_or_default();
            for config in configs {
                if let Err(e) = self
                    .services
                    .integrations
                    .indexer_configs
                    .delete(&config.id)
                    .await
                {
                    tracing::warn!(error = %e, indexer = config.name, "failed to delete indexer config during plugin uninstall");
                }
            }
        }

        self.services
            .customization
            .plugin_installations
            .delete_plugin_installation(plugin_id)
            .await?;

        self.apply_runtime_plugin_removal(&installation)?;
        self.finalize_runtime_plugin_mutation(&installation.plugin_type, installation.is_enabled)
            .await?;
        Ok(())
    }

    /// Toggle a plugin's enabled/disabled state.
    pub async fn toggle_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
        enabled: bool,
    ) -> AppResult<PluginInstallation> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let mut installation = self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not installed")))?;

        installation.is_enabled = enabled;
        installation.updated_at = Utc::now();

        let result = self
            .services
            .customization
            .plugin_installations
            .update_plugin_installation(&installation, None)
            .await?;

        if enabled {
            if result.is_builtin && result.source_kind == PluginSourceKind::Bundled {
                self.apply_runtime_builtin_restore(&result)?;
            } else {
                let runtime_plugin = self.load_runtime_plugin_for_installation(&result).await?;
                self.apply_runtime_plugin_upsert(&result, runtime_plugin)?;
            }
        } else {
            self.apply_runtime_plugin_removal(&result)?;
        }
        self.finalize_runtime_plugin_mutation(&installation.plugin_type, true)
            .await?;
        Ok(result)
    }

    /// Upgrade a non-builtin plugin to the latest registry version.
    pub async fn upgrade_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<PluginInstallation> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.validate_catalog_upgrade_request(plugin_id).await?;
        self.runtime
            .plugins
            .plugin_install_orchestrator
            .begin(&actor.id, plugin_id, PluginInstallOperationKind::Upgrade)
            .await
            .map_err(|_| Self::plugin_install_in_progress_error(plugin_id))?;
        let reporter = PluginInstallProgressReporter::new(self, &actor.id, plugin_id);
        let result = self.perform_catalog_upgrade(plugin_id, &reporter).await;
        match &result {
            Ok(_) => reporter.succeeded().await,
            Err(error) => reporter.failed(error).await,
        }
        result
    }

    /// List available community rule packs from the cached central catalog.
    pub async fn list_rule_pack_registry(
        &self,
        actor: &User,
    ) -> AppResult<Vec<RulePackRegistryEntry>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let catalog = self.load_rule_pack_catalog().await?;

        let current = current_scryer_version();
        Ok(catalog
            .rule_packs
            .into_iter()
            .map(RulePackRegistryEntry::from)
            .filter(|pack| {
                pack.min_scryer_version
                    .as_ref()
                    .and_then(|v| semver::Version::parse(v).ok())
                    .is_none_or(|min| current >= &min)
            })
            .collect())
    }

    /// Fetch a community rule pack by its registry ID.
    pub async fn fetch_rule_pack_templates(
        &self,
        actor: &User,
        pack_id: &str,
    ) -> AppResult<Vec<RulePackTemplate>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let packs = self.list_rule_pack_registry(actor).await?;
        let pack = packs
            .iter()
            .find(|p| p.id == pack_id)
            .ok_or_else(|| AppError::NotFound(format!("rule pack {pack_id}")))?;

        let outbound_http = plugin_http_client(PluginHttpClientProfile::RulePackFetch)?;
        let response = outbound_http
            .send(
                plugin_request_policy(format!("rule_pack:{pack_id}"), "rule_pack_fetch"),
                || {
                    outbound_http
                        .client()
                        .get(&pack.url)
                        .header("Accept", "application/json")
                },
            )
            .await
            .map_err(|error| match error {
                OutboundHttpError::RateLimited(rate_limited) => AppError::Repository(
                    match rate_limited.retry_after.filter(|delay| !delay.is_zero()) {
                        Some(delay) => format!(
                            "failed to fetch rule pack: rate limited, retry after {}s",
                            delay.as_secs()
                        ),
                        None => "failed to fetch rule pack: rate limited".to_string(),
                    },
                ),
                OutboundHttpError::Transport { source, .. } => {
                    AppError::Repository(format!("failed to fetch rule pack: {source}"))
                }
            })?;

        if !response.status().is_success() {
            return Err(AppError::Repository(format!(
                "rule pack fetch failed (HTTP {})",
                response.status()
            )));
        }

        let manifest: RulePackManifest = response
            .json()
            .await
            .map_err(|e| AppError::Repository(format!("invalid rule pack JSON: {e}")))?;

        Ok(manifest
            .rules
            .into_iter()
            .map(|r| RulePackTemplate {
                id: r.id,
                title: r.title,
                description: r.description,
                category: r.category,
                rego_source: r.rego_source,
                applied_facets: r.applied_facets,
            })
            .collect())
    }
}

// Builtin indexers with fixed endpoints still need a user-supplied API key,
// so they should not be auto-created during reconciliation.
fn should_skip_auto_created_indexer_config(provider_type: &str) -> bool {
    provider_type.eq_ignore_ascii_case("nzbgeek") || provider_type.eq_ignore_ascii_case("dognzb")
}

#[cfg(test)]
mod plugin_http_client_tests {
    use super::{PluginHttpClientProfile, plugin_http_client};

    #[test]
    fn plugin_http_client_profiles_are_cached() {
        let default_a = plugin_http_client(PluginHttpClientProfile::DefaultFetch)
            .expect("default plugin HTTP client should build") as *const _;
        let default_b = plugin_http_client(PluginHttpClientProfile::DefaultFetch)
            .expect("default plugin HTTP client should stay cached")
            as *const _;
        let rule_pack_a = plugin_http_client(PluginHttpClientProfile::RulePackFetch)
            .expect("rule-pack plugin HTTP client should build")
            as *const _;
        let rule_pack_b = plugin_http_client(PluginHttpClientProfile::RulePackFetch)
            .expect("rule-pack plugin HTTP client should stay cached")
            as *const _;

        assert_eq!(default_a, default_b);
        assert_eq!(rule_pack_a, rule_pack_b);
        assert_ne!(default_a, rule_pack_a);
    }
}

#[cfg(test)]
#[path = "app_usecase_plugins_tests.rs"]
mod app_usecase_plugins_tests;
