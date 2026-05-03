use super::catalog::{
    CentralCatalogEntry, ChildCatalog, ChildCatalogRelease, GitHubRepo, PluginReleaseManifest,
    RequiredSigner, blake3_digest, decompress_zstd, github_outage_status_from_summary,
    parse_and_validate_central_catalog, parse_and_validate_child_catalog,
    parse_and_validate_release_manifest, plugin_manifest_asset_url, verify_digest,
    verify_signed_blob,
};
use super::*;
use crate::ProviderCatalogFamily;
use chrono::Utc;
use ring::digest as ring_digest;
use scryer_domain::{PluginSourceKind, PluginSupportTier};
use scryer_plugin_sdk::{
    PluginDescriptor, SDK_VERSION, host_version_matches_constraint,
    load_plugin_descriptor_from_wasm_bytes, plugin_descriptor_sdk_constraint,
    sdk_constraint_or_legacy, validate_plugin_descriptor_host_permissions,
    validate_plugin_descriptor_sdk_contract, validate_sdk_contract,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::LazyLock};
use tracing::{debug, warn};

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

#[derive(Clone, Debug)]
struct CatalogPluginResolution {
    central: Option<CentralCatalogEntry>,
    child: ChildCatalog,
    release: ChildCatalogRelease,
    source_kind: PluginSourceKind,
    effective_support_tier: PluginSupportTier,
    github_repo: GitHubRepo,
}

/// Community rule pack entry from the registry.
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

/// Raw registry JSON format (matches scryer-plugins/registry.json).
#[derive(Clone, Debug, Deserialize)]
struct RegistryManifest {
    #[expect(dead_code)]
    schema_version: u32,
    plugins: Vec<RegistryEntry>,
    #[serde(default)]
    rule_packs: Vec<RulePackRegistryEntry>,
}

#[derive(Clone, Debug, Deserialize)]
struct RegistryEntry {
    id: String,
    name: String,
    description: String,
    plugin_type: String,
    provider_type: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    official: bool,
    #[serde(default)]
    releases: Vec<RegistryRelease>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    sdk_version: Option<String>,
    #[serde(default)]
    sdk_constraint: Option<String>,
    #[serde(default)]
    scryer_constraint: Option<String>,
    #[serde(default, rename = "min_scryer_version")]
    legacy_min_scryer_version: Option<String>,
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default)]
    wasm_url: Option<String>,
    #[serde(default)]
    wasm_sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct RegistryRelease {
    version: String,
    #[serde(default)]
    sdk_version: String,
    #[serde(default)]
    sdk_constraint: String,
    #[serde(default)]
    scryer_constraint: Option<String>,
    #[serde(default, rename = "min_scryer_version")]
    legacy_min_scryer_version: Option<String>,
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default)]
    wasm_url: Option<String>,
    #[serde(default)]
    wasm_sha256: Option<String>,
}

impl RegistryEntry {
    fn normalized_releases(&self) -> Vec<RegistryRelease> {
        if !self.releases.is_empty() {
            return self.releases.clone();
        }

        self.version
            .as_ref()
            .map(|version| {
                vec![RegistryRelease {
                    version: version.clone(),
                    sdk_version: self.sdk_version.clone().unwrap_or_default(),
                    sdk_constraint: self.sdk_constraint.clone().unwrap_or_default(),
                    scryer_constraint: self.scryer_constraint.clone(),
                    legacy_min_scryer_version: self.legacy_min_scryer_version.clone(),
                    source_url: self.source_url.clone(),
                    wasm_url: self.wasm_url.clone(),
                    wasm_sha256: self.wasm_sha256.clone(),
                }]
            })
            .unwrap_or_default()
    }
}

fn registry_release_scryer_constraint(release: &RegistryRelease) -> Option<&str> {
    release
        .scryer_constraint
        .as_deref()
        .or(release.legacy_min_scryer_version.as_deref())
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

fn normalized_release_sdk_constraint(release: &RegistryRelease) -> String {
    sdk_constraint_or_legacy(&release.sdk_version, &release.sdk_constraint)
}

fn parse_registry_release_version(
    plugin_id: &str,
    release: &RegistryRelease,
) -> Option<semver::Version> {
    semver::Version::parse(release.version.trim()).map_or_else(
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

fn parse_registry_release_sdk_req(
    plugin_id: &str,
    release: &RegistryRelease,
) -> Option<semver::VersionReq> {
    let sdk_version = release.sdk_version.trim();
    let descriptor_version = semver::Version::parse(sdk_version).map_or_else(
        |error| {
            warn!(
                plugin_id,
                version = release.version.as_str(),
                sdk_version,
                error = %error,
                "skipping plugin release with invalid sdk_version"
            );
            None
        },
        Some,
    )?;
    let constraint = normalized_release_sdk_constraint(release);
    let req = semver::VersionReq::parse(constraint.trim()).map_or_else(
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
    )?;
    if !req.matches(&descriptor_version) {
        warn!(
            plugin_id,
            version = release.version.as_str(),
            sdk_version,
            sdk_constraint = constraint.as_str(),
            "skipping plugin release whose sdk_version does not satisfy sdk_constraint"
        );
        return None;
    }
    Some(req)
}

fn registry_release_is_host_compatible(plugin_id: &str, release: &RegistryRelease) -> bool {
    let Some(sdk_req) = parse_registry_release_sdk_req(plugin_id, release) else {
        return false;
    };
    if !sdk_req.matches(current_sdk_version()) {
        return false;
    }
    let Some(constraint) = registry_release_scryer_constraint(release) else {
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

fn registry_release_has_valid_scryer_constraint(
    plugin_id: &str,
    release: &RegistryRelease,
) -> bool {
    let Some(constraint) = registry_release_scryer_constraint(release) else {
        return true;
    };
    let constraint = constraint.trim();
    if constraint.is_empty() {
        return true;
    }
    match semver::VersionReq::parse(constraint) {
        Ok(_) => true,
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

fn highest_registry_release<F>(entry: &RegistryEntry, predicate: F) -> Option<RegistryRelease>
where
    F: Fn(&RegistryRelease) -> bool,
{
    entry
        .normalized_releases()
        .into_iter()
        .filter(|release| predicate(release))
        .filter_map(|release| {
            let version = parse_registry_release_version(&entry.id, &release)?;
            parse_registry_release_sdk_req(&entry.id, &release)?;
            if !registry_release_has_valid_scryer_constraint(&entry.id, &release) {
                return None;
            }
            Some((version, release))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, release)| release)
}

fn latest_release(entry: &RegistryEntry) -> Option<RegistryRelease> {
    highest_registry_release(entry, |_| true)
}

fn latest_compatible_release(entry: &RegistryEntry) -> Option<RegistryRelease> {
    highest_registry_release(entry, |release| {
        registry_release_is_host_compatible(&entry.id, release)
    })
}

fn latest_compatible_child_release(child: &ChildCatalog) -> Option<ChildCatalogRelease> {
    let sdk_version = current_sdk_version();
    child
        .releases
        .iter()
        .filter(|release| {
            semver::VersionReq::parse(&release.sdk_constraint)
                .map(|req| req.matches(&sdk_version))
                .unwrap_or(false)
        })
        .filter_map(|release| {
            semver::Version::parse(release.version.trim_start_matches('v'))
                .ok()
                .map(|version| (version, release))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, release)| release.clone())
}

fn latest_host_blocked_release(entry: &RegistryEntry) -> Option<RegistryRelease> {
    let latest = latest_release(entry)?;
    let selected = latest_compatible_release(entry)?;
    let latest_version = parse_registry_release_version(&entry.id, &latest)?;
    let selected_version = parse_registry_release_version(&entry.id, &selected)?;
    if latest_version > selected_version && !registry_release_is_host_compatible(&entry.id, &latest)
    {
        Some(latest)
    } else {
        None
    }
}

fn installed_registry_release(
    entry: &RegistryEntry,
    installation: &PluginInstallation,
) -> Option<RegistryRelease> {
    entry.normalized_releases().into_iter().find(|release| {
        release.version == installation.version
            && normalized_release_sdk_constraint(release) == installation.sdk_constraint
            && release
                .wasm_sha256
                .as_deref()
                .zip(installation.wasm_sha256.as_deref())
                .is_some_and(|(expected, installed)| expected.eq_ignore_ascii_case(installed))
    })
}

fn installation_matches_official_registry(
    installation: &PluginInstallation,
    registry: Option<&RegistryManifest>,
) -> bool {
    let Some(registry) = registry else {
        return false;
    };

    registry.plugins.iter().any(|entry| {
        entry.official
            && entry.id == installation.plugin_id
            && entry.plugin_type == installation.plugin_type
            && entry.provider_type == installation.provider_type
            && installed_registry_release(entry, installation).is_some()
    })
}

fn installation_is_catalog_official(installation: &PluginInstallation) -> bool {
    installation.source_kind == PluginSourceKind::Downloaded
        && installation.support_tier == PluginSupportTier::Official
        && installation.wasm_digest.is_some()
}

fn installation_is_first_party(
    installation: &PluginInstallation,
    registry: Option<&RegistryManifest>,
) -> bool {
    installation_is_catalog_official(installation)
        || installation_matches_official_registry(installation, registry)
}

async fn installation_wasm_digest_is_valid(
    installation: &PluginInstallation,
    wasm_bytes: &[u8],
) -> bool {
    let Some(expected_digest) = installation.wasm_digest.clone() else {
        return true;
    };

    let plugin_id = installation.plugin_id.clone();
    let version = installation.version.clone();
    let bytes = wasm_bytes.to_vec();
    match tokio::task::spawn_blocking(move || blake3_digest(&bytes)).await {
        Ok(actual_digest) if actual_digest.eq_ignore_ascii_case(&expected_digest) => true,
        Ok(actual_digest) => {
            warn!(
                plugin_id = plugin_id.as_str(),
                version = version.as_str(),
                expected_digest = expected_digest.as_str(),
                actual_digest = actual_digest.as_str(),
                "skipping installed plugin with mismatched persisted wasm digest"
            );
            false
        }
        Err(error) => {
            warn!(
                plugin_id = plugin_id.as_str(),
                version = version.as_str(),
                error = %error,
                "skipping installed plugin after wasm digest verification failed"
            );
            false
        }
    }
}

fn installation_is_host_blocked_by_registry(
    installation: &PluginInstallation,
    registry: Option<&RegistryManifest>,
) -> bool {
    installation_scryer_constraint(installation, registry).is_some_and(|constraint| {
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

fn installation_scryer_constraint(
    installation: &PluginInstallation,
    registry: Option<&RegistryManifest>,
) -> Option<String> {
    normalized_constraint(installation.scryer_constraint.as_deref()).or_else(|| {
        registry
            .and_then(|registry| {
                registry.plugins.iter().find(|entry| {
                    entry.official
                        && entry.id == installation.plugin_id
                        && entry.plugin_type == installation.plugin_type
                        && entry.provider_type == installation.provider_type
                })
            })
            .and_then(|entry| installed_registry_release(entry, installation))
            .and_then(|release| normalized_constraint(registry_release_scryer_constraint(&release)))
    })
}

fn validate_downloaded_plugin_descriptor(
    plugin_id: &str,
    expected_plugin_type: &str,
    expected_provider_type: &str,
    release: &RegistryRelease,
    descriptor: &PluginDescriptor,
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
            "downloaded plugin '{}' has version '{}' but registry selected '{}'",
            descriptor.id, descriptor.version, release.version
        )));
    }
    if !release.sdk_version.trim().is_empty() && descriptor.sdk_version != release.sdk_version {
        return Err(AppError::Validation(format!(
            "downloaded plugin '{}' has sdk_version '{}' but registry selected '{}'",
            descriptor.id, descriptor.sdk_version, release.sdk_version
        )));
    }
    let descriptor_sdk_constraint = plugin_descriptor_sdk_constraint(descriptor);
    let release_sdk_constraint = normalized_release_sdk_constraint(release);
    if descriptor_sdk_constraint != release_sdk_constraint {
        warn!(
            plugin_id = descriptor.id.as_str(),
            version = release.version.as_str(),
            descriptor_sdk_constraint = descriptor_sdk_constraint.as_str(),
            registry_sdk_constraint = release_sdk_constraint.as_str(),
            "downloaded plugin sdk_constraint differs from registry metadata; using registry constraint"
        );
    }
    if !registry_release_is_host_compatible(plugin_id, release) {
        return Err(AppError::Validation(format!(
            "plugin '{}' no longer has a host-compatible release for this Scryer version",
            plugin_id
        )));
    }

    Ok(ValidatedDownloadedPlugin {
        descriptor: descriptor.clone(),
        sdk_constraint: release_sdk_constraint,
        scryer_constraint: normalized_constraint(registry_release_scryer_constraint(release)),
    })
}

const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/scryer-media/scryer-plugins/main/registry.json";
const REGISTRY_URL_ENV: &str = "SCRYER_PLUGIN_REGISTRY_URL";
const REGISTRY_PATH_ENV: &str = "SCRYER_PLUGIN_REGISTRY_PATH";
const DEFAULT_CATALOG_URL: &str = "https://github.com/scryer-media/scryer-plugins/releases/download/catalog%2Fv2/catalog-v2.min.json.zst";
const CATALOG_URL_ENV: &str = "SCRYER_PLUGIN_CATALOG_URL";
const GITHUB_STATUS_SUMMARY_URL: &str = "https://www.githubstatus.com/api/v2/summary.json";
const CENTRAL_CATALOG_SOURCE_KEY: &str = "__central_catalog_v2";
const CATALOG_STATUS_KEY: &str = "github_distribution";
const CENTRAL_CATALOG_REPO: &str = "scryer-media/scryer-plugins";
const CENTRAL_CATALOG_WORKFLOW: &str = ".github/workflows/release-plugin.yml";

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

fn plugin_registry_url() -> String {
    std::env::var(REGISTRY_URL_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_REGISTRY_URL.to_string())
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

fn plugin_registry_path_override() -> Option<PathBuf> {
    std::env::var(REGISTRY_PATH_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
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
    scryer_constraint: Option<String>,
}

async fn fetch_plugin_bytes(url: &str, label: &str) -> AppResult<Vec<u8>> {
    let response = reqwest::get(url)
        .await
        .map_err(|error| AppError::Repository(format!("failed to download {label}: {error}")))?
        .error_for_status()
        .map_err(|error| AppError::Repository(format!("failed to download {label}: {error}")))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|error| AppError::Repository(format!("failed to read {label}: {error}")))?;
    Ok(bytes.to_vec())
}

async fn download_registry_release_wasm(
    plugin_id: &str,
    release: &RegistryRelease,
) -> AppResult<Vec<u8>> {
    let wasm_url = release.wasm_url.as_ref().ok_or_else(|| {
        AppError::Validation(format!(
            "plugin '{plugin_id}' has no wasm_url in the selected registry release"
        ))
    })?;

    let wasm_bytes = fetch_plugin_bytes(wasm_url, "plugin WASM").await?;

    if let Some(expected_sha) = release.wasm_sha256.as_deref() {
        let actual_sha =
            crate::to_hex(ring_digest::digest(&ring_digest::SHA256, &wasm_bytes).as_ref());
        if actual_sha != expected_sha {
            return Err(AppError::Validation(format!(
                "WASM SHA256 mismatch: expected {expected_sha}, got {actual_sha}"
            )));
        }
    }

    Ok(wasm_bytes.to_vec())
}

fn no_compatible_registry_release_error(plugin_id: &str, entry: &RegistryEntry) -> AppError {
    if let Some(latest) = latest_release(entry) {
        if let Some(constraint) = registry_release_scryer_constraint(&latest) {
            return AppError::Validation(format!(
                "plugin '{plugin_id}' requires Scryer {constraint} but current Scryer is {CURRENT_SCRYER_VERSION}"
            ));
        }
        return AppError::Validation(format!(
            "plugin '{plugin_id}' has no compatible release for this Scryer version"
        ));
    }

    AppError::Validation(format!(
        "plugin '{plugin_id}' has no valid releases in the registry"
    ))
}

fn validate_downloaded_plugin_release(
    plugin_id: &str,
    expected_plugin_type: &str,
    expected_provider_type: &str,
    release: &RegistryRelease,
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

        let registry = self
            .services
            .customization
            .plugin_installations
            .get_registry_cache()
            .await?
            .and_then(|json| serde_json::from_str::<RegistryManifest>(&json).ok());

        // Downloaded overrides take precedence over bundled builtins.
        let mut external_bytes: Vec<(Vec<u8>, bool)> = Vec::new();
        for (inst, wasm) in enabled {
            if !matches!(
                inst.source_kind,
                PluginSourceKind::Downloaded | PluginSourceKind::Manual
            ) {
                continue;
            }
            if !installation_sdk_contract_is_host_compatible(&inst) {
                continue;
            }
            if installation_is_host_blocked_by_registry(&inst, registry.as_ref()) {
                continue;
            }

            let Some(bytes) = wasm else {
                continue;
            };
            if !installation_wasm_digest_is_valid(&inst, &bytes).await {
                continue;
            }

            external_bytes.push((bytes, installation_is_first_party(&inst, registry.as_ref())));
        }
        let external_refs: Vec<ExternalPluginWasm<'_>> = external_bytes
            .iter()
            .map(|(bytes, first_party)| ExternalPluginWasm {
                bytes: bytes.as_slice(),
                first_party: *first_party,
            })
            .collect();

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
            .filter(|inst| inst.is_builtin && !inst.is_enabled)
            .map(|inst| inst.provider_type.clone())
            .collect();

        if let Some(provider) = self.services.integrations.plugin_provider.available() {
            provider
                .reload_plugins(&external_refs, &disabled_builtins)
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
                .reload_plugins(&external_refs, &disabled_builtins)
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
                .reload_plugins(&external_refs, &disabled_builtins)
                .map_err(|e| {
                    AppError::Repository(format!("failed to reload subtitle plugin provider: {e}"))
                })?;
        }

        // Also rebuild notification plugin provider
        if let Some(notif_provider) = self.services.notifications.notification_provider() {
            notif_provider
                .reload_plugins(&external_refs, &disabled_builtins)
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
        self.rebuild_user_rules_engine().await?;
        Ok(())
    }

    /// Ensure every auto-provisionable indexer plugin with a `default_base_url`
    /// has at least one IndexerConfig. This covers the case where a plugin was
    /// installed before the auto-create logic existed, or when the registry was
    /// stale at install time.
    pub async fn reconcile_indexer_configs(&self) -> AppResult<()> {
        let Some(provider) = self.services.integrations.plugin_provider.available() else {
            return Ok(());
        };

        let now = Utc::now();
        for pt in provider.available_provider_types() {
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
                    base_url: default_url,
                    api_key_encrypted: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    rate_limit_seconds: provider.rate_limit_seconds_for_provider(&pt),
                    rate_limit_burst: None,
                    disabled_until: None,
                    last_health_status: None,
                    last_error_at: None,
                    config_json: None,
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
        require(actor, &Entitlement::ManageConfig)?;

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

    async fn build_available_plugins(&self) -> AppResult<Vec<RegistryPlugin>> {
        let installations = self
            .services
            .customization
            .plugin_installations
            .list_plugin_installations()
            .await?;

        // Try to parse cached registry
        let registry_json = self
            .services
            .customization
            .plugin_installations
            .get_registry_cache()
            .await?;

        let registry_entries: Vec<RegistryEntry> = match registry_json {
            Some(json) => serde_json::from_str::<RegistryManifest>(&json)
                .map(|m| m.plugins)
                .unwrap_or_default(),
            None => vec![],
        };
        let builtin_by_key = self.builtin_seed_by_key();
        let effective_installations = installations.iter().collect::<Vec<_>>();

        let mut result = Vec::new();

        for resolved in self.resolved_catalog_plugins().await.unwrap_or_default() {
            let inst = effective_installations
                .iter()
                .copied()
                .find(|installation| installation.plugin_id == resolved.child.id);
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
                wasm_sha256: inst.and_then(|installation| installation.wasm_sha256.clone()),
                min_scryer_version: None,
                default_base_url: self
                    .default_base_url_for_plugin(&plugin_type, &resolved.child.provider_type),
                is_installed: inst.is_some(),
                is_enabled: inst.map(|i| i.is_enabled).unwrap_or(false),
                installed_version: inst.map(|i| i.version.clone()),
                update_available,
            });
        }

        for entry in &registry_entries {
            if result.iter().any(|plugin| plugin.id == entry.id) {
                continue;
            }
            let inst = effective_installations
                .iter()
                .copied()
                .find(|installation| installation.plugin_id == entry.id);

            let plugin_type =
                merged_plugin_type(&entry.plugin_type, inst.map(|i| i.plugin_type.as_str()));
            let builtin_key = builtin_lookup_key(&plugin_type, &entry.provider_type);
            let builtin_seed = builtin_by_key.get(&builtin_key);
            let selected_release = latest_compatible_release(entry);
            let latest_release = latest_release(entry);
            let blocked_release = latest_host_blocked_release(entry);
            let active_release =
                inst.and_then(|installation| installed_registry_release(entry, installation));
            let display_release = selected_release
                .clone()
                .or_else(|| active_release.clone())
                .or_else(|| latest_release.clone());

            if inst.is_none() && display_release.is_none() && builtin_seed.is_none() {
                continue;
            }

            let version = display_release
                .as_ref()
                .map(|release| release.version.clone())
                .or_else(|| inst.map(|installation| installation.version.clone()))
                .unwrap_or_default();
            let latest_version = match (selected_release.as_ref(), latest_release.as_ref()) {
                (Some(selected), Some(latest)) => {
                    let selected_version = parse_registry_release_version(&entry.id, selected);
                    let latest_semver = parse_registry_release_version(&entry.id, latest);
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
                        parse_registry_release_version(&entry.id, release)
                            .zip(semver::Version::parse(installation.version.as_str()).ok())
                    })
                })
                .is_some_and(|(selected_version, installed_version)| {
                    selected_version > installed_version
                });
            let release_source_url = display_release
                .as_ref()
                .and_then(|release| release.source_url.clone());
            let release_wasm_url = display_release
                .as_ref()
                .and_then(|release| release.wasm_url.clone());
            let release_wasm_sha256 = display_release
                .as_ref()
                .and_then(|release| release.wasm_sha256.clone());
            let release_scryer_constraint = display_release.as_ref().and_then(|release| {
                registry_release_scryer_constraint(release).map(str::to_string)
            });
            let builtin = inst
                .map(|installation| installation.is_builtin)
                .unwrap_or_else(|| builtin_seed.is_some());

            result.push(RegistryPlugin {
                id: entry.id.clone(),
                name: entry.name.clone(),
                description: entry.description.clone(),
                version,
                latest_version,
                plugin_type,
                provider_type: entry.provider_type.clone(),
                author: entry.author.clone(),
                official: entry.official,
                publisher: inst
                    .and_then(|installation| installation.publisher.clone())
                    .or_else(|| Some(entry.author.clone())),
                support_tier: inst
                    .map(|installation| installation.support_tier)
                    .unwrap_or(PluginSupportTier::Official),
                docs_url: inst.and_then(|installation| installation.docs_url.clone()),
                source_repo: inst.and_then(|installation| installation.source_repo.clone()),
                builtin,
                source_url: inst
                    .and_then(|installation| installation.source_url.clone())
                    .or(release_source_url),
                source_kind: inst.map(|installation| source_kind_label(installation.source_kind)),
                blocked_reason,
                wasm_url: release_wasm_url,
                wasm_sha256: release_wasm_sha256,
                min_scryer_version: release_scryer_constraint,
                default_base_url: self
                    .default_base_url_for_plugin(&entry.plugin_type, &entry.provider_type),
                is_installed: inst.is_some(),
                is_enabled: inst.map(|i| i.is_enabled).unwrap_or(false),
                installed_version: inst.map(|i| i.version.clone()),
                update_available,
            });
        }

        for inst in effective_installations {
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
                    wasm_sha256: inst.wasm_sha256.clone(),
                    min_scryer_version: None,
                    default_base_url: self
                        .default_base_url_for_plugin(&inst.plugin_type, &inst.provider_type),
                    is_installed: true,
                    is_enabled: inst.is_enabled,
                    installed_version: Some(inst.version.clone()),
                    update_available: false,
                });
            }
        }

        Ok(result)
    }

    /// List available plugins by merging cached registry with local installations.
    pub async fn list_available_plugins(&self, actor: &User) -> AppResult<Vec<RegistryPlugin>> {
        require(actor, &Entitlement::ManageConfig)?;

        self.build_available_plugins().await
    }

    pub async fn plugin_update_count(&self, actor: &User) -> AppResult<i64> {
        require(actor, &Entitlement::ManageConfig)?;

        Ok(self
            .build_available_plugins()
            .await?
            .into_iter()
            .filter(|plugin| plugin.update_available)
            .count() as i64)
    }

    async fn synchronize_plugin_installation_release_metadata(
        &self,
        manifest: &RegistryManifest,
    ) -> AppResult<()> {
        let repo = &self.services.customization.plugin_installations;

        for mut installation in repo
            .list_plugin_installations()
            .await?
            .into_iter()
            .filter(|installation| installation.source_kind == PluginSourceKind::Downloaded)
        {
            let Some(entry) = manifest.plugins.iter().find(|entry| {
                entry.official
                    && entry.id == installation.plugin_id
                    && entry.plugin_type == installation.plugin_type
                    && entry.provider_type == installation.provider_type
            }) else {
                continue;
            };

            let Some(release) = installed_registry_release(entry, &installation) else {
                continue;
            };

            let next_sdk_constraint = normalized_release_sdk_constraint(&release);
            let next_scryer_constraint =
                normalized_constraint(registry_release_scryer_constraint(&release));
            let next_source_url = release.source_url.clone();
            let next_wasm_sha256 = release.wasm_sha256.clone();

            if installation.sdk_constraint == next_sdk_constraint
                && installation.scryer_constraint == next_scryer_constraint
                && installation.source_url == next_source_url
                && installation.wasm_sha256 == next_wasm_sha256
            {
                continue;
            }

            installation.sdk_constraint = next_sdk_constraint;
            installation.scryer_constraint = next_scryer_constraint;
            installation.source_url = next_source_url;
            installation.wasm_sha256 = next_wasm_sha256;
            installation.updated_at = Utc::now();
            repo.update_plugin_installation(&installation, None).await?;
        }

        Ok(())
    }

    async fn apply_plugin_registry_manifest(
        &self,
        manifest: &RegistryManifest,
        body: &str,
    ) -> AppResult<()> {
        self.services
            .customization
            .plugin_installations
            .store_registry_cache(body)
            .await?;
        self.synchronize_plugin_installation_release_metadata(manifest)
            .await?;
        self.rebuild_plugin_provider().await?;
        self.publish_provider_catalog_changed(ProviderCatalogFamily::all().into_iter().collect());
        Ok(())
    }

    /// Refresh the plugin registry from the remote URL.
    pub async fn refresh_plugin_registry(&self, actor: &User) -> AppResult<Vec<RegistryPlugin>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.refresh_plugin_registry_internal().await?;
        self.list_available_plugins(actor).await
    }

    /// Internal registry refresh (no auth check) for use by startup and background tasks.
    pub async fn refresh_plugin_registry_internal(&self) -> AppResult<()> {
        let body = if let Some(path) = plugin_registry_path_override() {
            match std::fs::read_to_string(&path) {
                Ok(body) => body,
                Err(error) => {
                    warn!(
                        path = %path.display(),
                        error = %error,
                        "failed to read local plugin registry override; falling back to remote registry"
                    );
                    let url = plugin_registry_url();
                    reqwest::get(&url)
                        .await
                        .map_err(|e| {
                            AppError::Repository(format!("failed to fetch plugin registry: {e}"))
                        })?
                        .text()
                        .await
                        .map_err(|e| {
                            AppError::Repository(format!(
                                "failed to read plugin registry body: {e}"
                            ))
                        })?
                }
            }
        } else {
            let url = plugin_registry_url();
            reqwest::get(&url)
                .await
                .map_err(|e| AppError::Repository(format!("failed to fetch plugin registry: {e}")))?
                .text()
                .await
                .map_err(|e| {
                    AppError::Repository(format!("failed to read plugin registry body: {e}"))
                })?
        };

        let manifest: RegistryManifest = serde_json::from_str(&body)
            .map_err(|e| AppError::Validation(format!("invalid plugin registry JSON: {e}")))?;

        self.apply_plugin_registry_manifest(&manifest, &body).await
    }

    pub async fn plugin_catalog_status(&self, actor: &User) -> AppResult<PluginCatalogStatus> {
        require(actor, &Entitlement::ManageConfig)?;

        let now = Utc::now();
        let status = match fetch_plugin_bytes(GITHUB_STATUS_SUMMARY_URL, "GitHub Status").await {
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
        require(actor, &Entitlement::ManageConfig)?;
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

        for entry in &central.plugins {
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
                parse_and_validate_child_catalog(&child_raw, Some(entry), None)?;
                String::from_utf8(child_raw).map_err(|e| {
                    AppError::Validation(format!("child plugin catalog is not UTF-8: {e}"))
                })
            }
            .await;

            match result {
                Ok(child_json) => {
                    self.services
                        .customization
                        .plugin_installations
                        .upsert_plugin_catalog_source(&PluginCatalogSource {
                            source_key,
                            source_kind: "child".to_string(),
                            source_url: entry.child_catalog_url.clone(),
                            github_repo: Some(source_repo.slug()),
                            support_tier: entry.support_tier,
                            catalog_json: Some(child_json),
                            last_success_at: Some(now),
                            last_error: None,
                            updated_at: now,
                        })
                        .await?;
                }
                Err(error) => {
                    warn!(
                        plugin_id = entry.id.as_str(),
                        error = %error,
                        "verified child plugin catalog is unavailable"
                    );
                    self.services
                        .customization
                        .plugin_installations
                        .upsert_plugin_catalog_source(&PluginCatalogSource {
                            source_key,
                            source_kind: "child".to_string(),
                            source_url: entry.child_catalog_url.clone(),
                            github_repo: Some(source_repo.slug()),
                            support_tier: entry.support_tier,
                            catalog_json: None,
                            last_success_at: None,
                            last_error: Some(error.to_string()),
                            updated_at: now,
                        })
                        .await?;
                }
            }
        }

        self.publish_provider_catalog_changed(ProviderCatalogFamily::all().into_iter().collect());
        Ok(())
    }

    async fn fetch_verified_catalog_bytes(
        &self,
        url: &str,
        signer: &RequiredSigner,
        label: &str,
    ) -> AppResult<Vec<u8>> {
        let raw = fetch_plugin_bytes(url, label).await?;
        let bundle = fetch_plugin_bytes(&bundle_url_for(url), &format!("{label} bundle")).await?;
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
    ) -> AppResult<(Vec<u8>, PluginReleaseManifest, String)> {
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
        let artifact = fetch_plugin_bytes(&artifact_url, "plugin artifact").await?;
        let artifact_bundle =
            fetch_plugin_bytes(&artifact_bundle_url, "plugin artifact bundle").await?;
        verify_signed_blob(artifact.clone(), artifact_bundle, signer).await?;
        verify_digest(
            "compressed plugin artifact",
            &manifest.artifact_digest,
            &artifact,
        )?;
        let wasm = decompress_zstd(artifact).await?;
        verify_digest("decompressed plugin WASM", &manifest.wasm_digest, &wasm)?;
        Ok((wasm, manifest, artifact_url))
    }

    async fn install_catalog_plugin(
        &self,
        resolved: CatalogPluginResolution,
    ) -> AppResult<PluginInstallation> {
        let (wasm_bytes, manifest, artifact_url) =
            self.fetch_catalog_release_wasm(&resolved).await?;
        let registry_release = RegistryRelease {
            version: resolved.release.version.clone(),
            sdk_version: String::new(),
            sdk_constraint: resolved.release.sdk_constraint.clone(),
            scryer_constraint: None,
            legacy_min_scryer_version: None,
            source_url: Some(resolved.child.source_repo.clone()),
            wasm_url: Some(artifact_url.clone()),
            wasm_sha256: None,
        };
        let validated = validate_downloaded_plugin_release(
            &resolved.child.id,
            &resolved.child.plugin_type,
            &resolved.child.provider_type,
            &registry_release,
            &wasm_bytes,
        )?;

        let wasm_sha256 =
            crate::to_hex(ring_digest::digest(&ring_digest::SHA256, &wasm_bytes).as_ref());
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
            wasm_sha256: Some(wasm_sha256),
            source_url: Some(artifact_url),
            support_tier: resolved.effective_support_tier,
            publisher: Some(resolved.child.publisher.clone()),
            docs_url: Some(resolved.child.docs_url.clone()),
            source_repo: Some(resolved.child.source_repo.clone()),
            manifest_url: Some(resolved.release.artifact_manifest_url.clone()),
            wasm_digest: Some(manifest.wasm_digest),
            artifact_digest: Some(manifest.artifact_digest),
            installed_at: now,
            updated_at: now,
        };

        let result = self
            .services
            .customization
            .plugin_installations
            .create_plugin_installation(&installation, Some(wasm_bytes.as_slice()))
            .await?;

        self.rebuild_plugin_provider().await?;
        self.publish_provider_catalog_changed(ProviderCatalogFamily::all().into_iter().collect());
        Ok(result)
    }

    async fn upgrade_catalog_plugin(
        &self,
        resolved: CatalogPluginResolution,
        installation: PluginInstallation,
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

        let (wasm_bytes, manifest, artifact_url) =
            self.fetch_catalog_release_wasm(&resolved).await?;
        let registry_release = RegistryRelease {
            version: resolved.release.version.clone(),
            sdk_version: String::new(),
            sdk_constraint: resolved.release.sdk_constraint.clone(),
            scryer_constraint: None,
            legacy_min_scryer_version: None,
            source_url: Some(resolved.child.source_repo.clone()),
            wasm_url: Some(artifact_url.clone()),
            wasm_sha256: None,
        };
        let validated = validate_downloaded_plugin_release(
            &resolved.child.id,
            &resolved.child.plugin_type,
            &resolved.child.provider_type,
            &registry_release,
            &wasm_bytes,
        )?;

        let wasm_sha256 =
            crate::to_hex(ring_digest::digest(&ring_digest::SHA256, &wasm_bytes).as_ref());
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
        updated.wasm_sha256 = Some(wasm_sha256);
        updated.source_url = Some(artifact_url);
        updated.support_tier = resolved.effective_support_tier;
        updated.publisher = Some(resolved.child.publisher.clone());
        updated.docs_url = Some(resolved.child.docs_url.clone());
        updated.source_repo = Some(resolved.child.source_repo.clone());
        updated.manifest_url = Some(resolved.release.artifact_manifest_url.clone());
        updated.wasm_digest = Some(manifest.wasm_digest);
        updated.artifact_digest = Some(manifest.artifact_digest);
        updated.updated_at = Utc::now();

        let result = self
            .services
            .customization
            .plugin_installations
            .update_plugin_installation(&updated, Some(wasm_bytes.as_slice()))
            .await?;

        self.rebuild_plugin_provider().await?;
        self.publish_provider_catalog_changed(provider_catalog_families_for_plugin_type(
            &updated.plugin_type,
        ));
        Ok(result)
    }

    async fn resolve_manual_plugin_repo(
        &self,
        github_repo_url: &str,
    ) -> AppResult<(CatalogPluginResolution, String)> {
        let repo = GitHubRepo::parse(github_repo_url)?;
        let child_url = repo.child_catalog_url();
        let signer = RequiredSigner {
            github_repository: repo.slug(),
            github_workflow: None,
        };
        let child_raw = self
            .fetch_verified_catalog_bytes(&child_url, &signer, "manual plugin child catalog")
            .await?;
        let child = parse_and_validate_child_catalog(&child_raw, None, Some(&repo))?;
        let release = latest_compatible_child_release(&child).ok_or_else(|| {
            AppError::Validation(format!(
                "manual plugin repo '{}' has no SDK-compatible release",
                repo.slug()
            ))
        })?;
        let manifest_raw = self
            .fetch_verified_catalog_bytes(
                &release.artifact_manifest_url,
                &signer,
                "manual plugin release manifest",
            )
            .await?;
        parse_and_validate_release_manifest(&manifest_raw, &child, &release, &repo)?;
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
        require(actor, &Entitlement::ManageConfig)?;
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
        require(actor, &Entitlement::ManageConfig)?;
        let (resolved, child_json) = self.resolve_manual_plugin_repo(github_repo_url).await?;
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
        let now = Utc::now();
        self.services
            .customization
            .plugin_installations
            .upsert_plugin_catalog_source(&PluginCatalogSource {
                source_key: manual_catalog_source_key(&resolved.github_repo),
                source_kind: "manual".to_string(),
                source_url: resolved.github_repo.child_catalog_url(),
                github_repo: Some(resolved.github_repo.slug()),
                support_tier: PluginSupportTier::Unverified,
                catalog_json: Some(child_json),
                last_success_at: Some(now),
                last_error: None,
                updated_at: now,
            })
            .await?;
        self.install_catalog_plugin(resolved).await
    }

    /// Install a plugin from the registry.
    pub async fn install_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<PluginInstallation> {
        require(actor, &Entitlement::ManageConfig)?;
        let _operation_guard = self
            .runtime
            .plugins
            .plugin_operation_guards
            .acquire(plugin_id)
            .await;

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
            return self.install_catalog_plugin(resolved).await;
        }

        let registry_json = self
            .services
            .customization
            .plugin_installations
            .get_registry_cache()
            .await?
            .ok_or_else(|| {
                AppError::Validation("plugin registry not loaded; refresh first".to_string())
            })?;

        let manifest: RegistryManifest = serde_json::from_str(&registry_json)
            .map_err(|e| AppError::Repository(format!("invalid cached registry: {e}")))?;

        let entry = manifest
            .plugins
            .iter()
            .find(|p| p.id == plugin_id)
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not in registry")))?;
        let selected_release = latest_compatible_release(entry)
            .ok_or_else(|| no_compatible_registry_release_error(plugin_id, entry))?;
        let wasm_bytes = download_registry_release_wasm(plugin_id, &selected_release).await?;
        let validated = validate_downloaded_plugin_release(
            plugin_id,
            &entry.plugin_type,
            &entry.provider_type,
            &selected_release,
            &wasm_bytes,
        )?;

        let now = Utc::now();
        let installation = PluginInstallation {
            id: Id::new().0,
            plugin_id: plugin_id.to_string(),
            name: validated.descriptor.name.clone(),
            description: entry.description.clone(),
            version: validated.descriptor.version.clone(),
            sdk_version: validated.descriptor.sdk_version.clone(),
            sdk_constraint: validated.sdk_constraint.clone(),
            scryer_constraint: validated.scryer_constraint.clone(),
            plugin_type: validated.descriptor.plugin_type().to_string(),
            provider_type: normalize_provider_key(validated.descriptor.provider_type()),
            source_kind: PluginSourceKind::Downloaded,
            is_enabled: true,
            is_builtin: false,
            wasm_sha256: selected_release.wasm_sha256.clone(),
            source_url: selected_release.source_url.clone(),
            support_tier: scryer_domain::PluginSupportTier::Official,
            publisher: Some(entry.author.clone()),
            docs_url: None,
            source_repo: None,
            manifest_url: None,
            wasm_digest: None,
            artifact_digest: None,
            installed_at: now,
            updated_at: now,
        };

        let result = self
            .services
            .customization
            .plugin_installations
            .create_plugin_installation(&installation, Some(wasm_bytes.as_slice()))
            .await?;

        self.rebuild_plugin_provider().await?;

        // Auto-create an IndexerConfig for single-endpoint indexer plugins.
        // Read default_base_url from the loaded plugin descriptor (not the
        // registry cache) — the WASM itself is the source of truth.
        if is_indexer_plugin_type(&installation.plugin_type) {
            let default_url = self
                .services
                .integrations
                .plugin_provider
                .available()
                .and_then(|p| p.default_base_url_for_provider(&installation.provider_type));
            if let Some(ref default_url) = default_url {
                let existing = self
                    .services
                    .integrations
                    .indexer_configs
                    .list(Some(installation.provider_type.clone()))
                    .await
                    .unwrap_or_default();
                if existing.is_empty() {
                    let plugin_rate_limit = self
                        .services
                        .integrations
                        .plugin_provider
                        .available()
                        .and_then(|p| {
                            p.rate_limit_seconds_for_provider(&installation.provider_type)
                        });
                    let config = IndexerConfig {
                        id: Id::new().0,
                        name: installation.name.clone(),
                        provider_type: installation.provider_type.clone(),
                        base_url: default_url.clone(),
                        api_key_encrypted: None,
                        is_enabled: true,
                        enable_interactive_search: true,
                        enable_auto_search: true,
                        rate_limit_seconds: plugin_rate_limit,
                        rate_limit_burst: None,
                        disabled_until: None,
                        last_health_status: None,
                        last_error_at: None,
                        config_json: None,
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
                        tracing::warn!(error = %e, "failed to auto-create indexer config for plugin");
                    }
                }
            }
        }

        self.publish_provider_catalog_changed(provider_catalog_families_for_plugin_type(
            &installation.plugin_type,
        ));

        Ok(result)
    }

    /// Uninstall a non-builtin plugin or revert a downloaded builtin override.
    pub async fn uninstall_plugin(&self, actor: &User, plugin_id: &str) -> AppResult<()> {
        require(actor, &Entitlement::ManageConfig)?;

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
            reverted.wasm_sha256 = None;
            reverted.source_url = None;
            reverted.updated_at = Utc::now();

            self.services
                .customization
                .plugin_installations
                .update_plugin_installation(&reverted, None)
                .await?;

            self.rebuild_plugin_provider().await?;
            self.publish_provider_catalog_changed(provider_catalog_families_for_plugin_type(
                &reverted.plugin_type,
            ));
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

        self.rebuild_plugin_provider().await?;
        self.publish_provider_catalog_changed(provider_catalog_families_for_plugin_type(
            &installation.plugin_type,
        ));
        Ok(())
    }

    /// Toggle a plugin's enabled/disabled state.
    pub async fn toggle_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
        enabled: bool,
    ) -> AppResult<PluginInstallation> {
        require(actor, &Entitlement::ManageConfig)?;

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

        self.rebuild_plugin_provider().await?;
        self.publish_provider_catalog_changed(provider_catalog_families_for_plugin_type(
            &installation.plugin_type,
        ));
        Ok(result)
    }

    /// Upgrade a non-builtin plugin to the latest registry version.
    pub async fn upgrade_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<PluginInstallation> {
        require(actor, &Entitlement::ManageConfig)?;
        let _operation_guard = self
            .runtime
            .plugins
            .plugin_operation_guards
            .acquire(plugin_id)
            .await;

        let installation = self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not installed")))?;

        if let Some(resolved) = self
            .resolved_catalog_plugins()
            .await?
            .into_iter()
            .find(|plugin| plugin.child.id == plugin_id)
        {
            return self.upgrade_catalog_plugin(resolved, installation).await;
        }

        let registry_json = self
            .services
            .customization
            .plugin_installations
            .get_registry_cache()
            .await?
            .ok_or_else(|| {
                AppError::Validation("plugin registry not loaded; refresh first".to_string())
            })?;

        let manifest: RegistryManifest = serde_json::from_str(&registry_json)
            .map_err(|e| AppError::Repository(format!("invalid cached registry: {e}")))?;

        let entry = manifest
            .plugins
            .iter()
            .find(|p| p.id == plugin_id)
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not in registry")))?;
        let selected_release = latest_compatible_release(entry)
            .ok_or_else(|| no_compatible_registry_release_error(plugin_id, entry))?;

        let reg_ver = semver::Version::parse(&selected_release.version).map_err(|e| {
            AppError::Validation(format!(
                "invalid registry version '{}': {e}",
                selected_release.version
            ))
        })?;
        let inst_ver = semver::Version::parse(&installation.version).map_err(|e| {
            AppError::Validation(format!(
                "invalid installed version '{}': {e}",
                installation.version
            ))
        })?;
        if reg_ver <= inst_ver {
            return Err(AppError::Validation(format!(
                "plugin '{plugin_id}' is already at version {} (selected release is {})",
                installation.version, selected_release.version
            )));
        }
        let wasm_bytes = download_registry_release_wasm(plugin_id, &selected_release).await?;
        let validated = validate_downloaded_plugin_release(
            plugin_id,
            &entry.plugin_type,
            &entry.provider_type,
            &selected_release,
            &wasm_bytes,
        )?;

        let mut updated = installation;
        updated.version = validated.descriptor.version.clone();
        updated.name = validated.descriptor.name.clone();
        updated.description = entry.description.clone();
        updated.sdk_version = validated.descriptor.sdk_version.clone();
        updated.sdk_constraint = validated.sdk_constraint.clone();
        updated.scryer_constraint = validated.scryer_constraint.clone();
        updated.plugin_type = validated.descriptor.plugin_type().to_string();
        updated.provider_type = normalize_provider_key(validated.descriptor.provider_type());
        updated.source_kind = PluginSourceKind::Downloaded;
        updated.wasm_sha256 = selected_release.wasm_sha256.clone();
        updated.source_url = selected_release.source_url.clone();
        updated.updated_at = Utc::now();

        let result = self
            .services
            .customization
            .plugin_installations
            .update_plugin_installation(&updated, Some(wasm_bytes.as_slice()))
            .await?;

        self.rebuild_plugin_provider().await?;
        self.publish_provider_catalog_changed(provider_catalog_families_for_plugin_type(
            &updated.plugin_type,
        ));
        Ok(result)
    }

    /// List available community rule packs from the cached registry.
    pub async fn list_rule_pack_registry(
        &self,
        actor: &User,
    ) -> AppResult<Vec<RulePackRegistryEntry>> {
        require(actor, &Entitlement::ManageConfig)?;

        let registry_json = self
            .services
            .customization
            .plugin_installations
            .get_registry_cache()
            .await?;

        let Some(json) = registry_json else {
            return Ok(Vec::new());
        };

        let manifest: RegistryManifest = serde_json::from_str(&json)
            .map_err(|e| AppError::Repository(format!("failed to parse registry cache: {e}")))?;

        // Filter by min_scryer_version compatibility
        let current = current_scryer_version();
        Ok(manifest
            .rule_packs
            .into_iter()
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
        require(actor, &Entitlement::ManageConfig)?;

        // Find the pack URL from registry
        let packs = self.list_rule_pack_registry(actor).await?;
        let pack = packs
            .iter()
            .find(|p| p.id == pack_id)
            .ok_or_else(|| AppError::NotFound(format!("rule pack {pack_id}")))?;

        // Fetch the JSON
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| AppError::Repository(format!("failed to build HTTP client: {e}")))?;

        let response = http
            .get(&pack.url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| AppError::Repository(format!("failed to fetch rule pack: {e}")))?;

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
#[path = "app_usecase_plugins_tests.rs"]
mod app_usecase_plugins_tests;
