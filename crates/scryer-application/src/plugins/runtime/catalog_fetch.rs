#[derive(Clone, Debug)]
struct CatalogPluginResolution {
    catalog_entry: CatalogV3PluginEntry,
    release: CatalogV3PluginRelease,
    artifact: CatalogV3PluginArtifact,
    source_kind: PluginSourceKind,
    effective_support_tier: PluginSupportTier,
    github_repo: GitHubRepo,
}
struct PreparedCatalogPluginInstall {
    descriptor: PluginDescriptor,
    sdk_constraint: String,
    source_kind: PluginSourceKind,
    support_tier: PluginSupportTier,
    persisted_wasm_bytes: Vec<u8>,
    runtime_wasm_bytes: Vec<u8>,
    runtime_first_party: bool,
    wasm_encoding: PluginWasmEncoding,
    wasm_digest_algo: String,
    source_url: String,
    publisher: String,
    docs_url: String,
    source_repo: String,
    manifest_url: String,
    wasm_digest: String,
    artifact_digest: String,
    description: String,
}
/// Community rule pack entry from the official catalog.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RulePackRegistryEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub version: String,
    pub source_url: String,
    #[serde(default)]
    pub min_scryer_version: Option<String>,
}
impl RulePackRegistryEntry {
    fn from_release(
        value: &CatalogV3RulePackEntry,
        release: &CatalogV3RulePackRelease,
        artifact: &CatalogV3DistributionArtifact,
    ) -> Self {
        Self {
            id: value.id.clone(),
            name: value.name.clone(),
            description: value.description.clone(),
            author: value.author.clone(),
            version: release.version.clone(),
            source_url: artifact.url.clone(),
            min_scryer_version: release.min_scryer_version.clone(),
        }
    }
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
struct FetchedCatalogArtifact {
    persisted_wasm_bytes: Vec<u8>,
    wasm_bytes: Vec<u8>,
    artifact_url: String,
    artifact_digest: String,
    wasm_encoding: PluginWasmEncoding,
}
fn parse_catalog_release_version(
    plugin_id: &str,
    release: &CatalogV3PluginRelease,
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
fn parse_catalog_release_sdk_req(
    plugin_id: &str,
    release: &CatalogV3PluginRelease,
) -> Option<semver::VersionReq> {
    let constraint = effective_host_sdk_constraint(None, &release.sdk_constraint);
    semver::VersionReq::parse(constraint.trim()).map_or_else(
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
    )
}
fn catalog_release_is_sdk_compatible(plugin_id: &str, release: &CatalogV3PluginRelease) -> bool {
    let Some(sdk_req) = parse_catalog_release_sdk_req(plugin_id, release) else {
        return false;
    };
    sdk_req.matches(current_sdk_version())
}
fn latest_catalog_release(plugin: &CatalogV3PluginEntry) -> Option<CatalogV3PluginRelease> {
    plugin
        .releases
        .iter()
        .filter_map(|release| {
            parse_catalog_release_version(&plugin.id, release).map(|version| (version, release))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, release)| release.clone())
}
fn latest_compatible_catalog_release(
    plugin: &CatalogV3PluginEntry,
    supported_features: &HashSet<String>,
) -> Option<CatalogV3PluginRelease> {
    plugin
        .releases
        .iter()
        .filter(|release| catalog_release_is_sdk_compatible(&plugin.id, release))
        .filter(|release| {
            select_catalog_release_artifact(
                release,
                supported_features,
                crate::services::RuntimePerformanceClass::Slow,
            )
            .is_some()
        })
        .filter_map(|release| {
            parse_catalog_release_version(&plugin.id, release).map(|version| (version, release))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, release)| release.clone())
}
fn latest_host_blocked_catalog_release(
    plugin: &CatalogV3PluginEntry,
    supported_features: &HashSet<String>,
) -> Option<CatalogV3PluginRelease> {
    let latest = latest_catalog_release(plugin)?;
    let latest_version = parse_catalog_release_version(&plugin.id, &latest)?;
    let selected = latest_compatible_catalog_release(plugin, supported_features);
    match selected {
        Some(selected) => {
            let selected_version = parse_catalog_release_version(&plugin.id, &selected)?;
            if latest_version > selected_version
                && !catalog_release_is_sdk_compatible(&plugin.id, &latest)
            {
                Some(latest)
            } else {
                None
            }
        }
        None if !catalog_release_is_sdk_compatible(&plugin.id, &latest) => Some(latest),
        None => None,
    }
}
#[cfg(test)]
fn latest_compatible_child_release(child: &ChildCatalog) -> Option<ChildCatalogRelease> {
    child
        .releases
        .iter()
        .filter_map(|release| {
            let constraint = effective_host_sdk_constraint(None, &release.sdk_constraint);
            let sdk_req = semver::VersionReq::parse(constraint.trim()).ok()?;
            sdk_req.matches(current_sdk_version()).then_some(release)
        })
        .filter_map(|release| {
            semver::Version::parse(release.version.trim_start_matches('v'))
                .ok()
                .map(|version| (version, release))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, release)| release.clone())
}
fn installed_catalog_release(
    plugin: &CatalogV3PluginEntry,
    installation: &PluginInstallation,
) -> Option<CatalogV3PluginRelease> {
    plugin
        .releases
        .iter()
        .find(|release| {
            release.version == installation.version
                && release.sdk_constraint == installation.sdk_constraint
        })
        .cloned()
}
fn artifact_required_features_supported(
    artifact: &CatalogV3PluginArtifact,
    supported_features: &HashSet<String>,
) -> bool {
    artifact.runtime == CATALOG_V3_RUNTIME_WASIP1
        && artifact
            .required_features
            .iter()
            .all(|feature| supported_features.contains(&feature.trim().to_ascii_lowercase()))
}
fn artifact_feature_specificity(artifact: &CatalogV3PluginArtifact) -> usize {
    artifact.required_features.len()
}
fn select_catalog_release_artifact(
    release: &CatalogV3PluginRelease,
    supported_features: &HashSet<String>,
    cpu_class: crate::services::RuntimePerformanceClass,
) -> Option<CatalogV3PluginArtifact> {
    let preferred_encoding = preferred_plugin_artifact_encoding(cpu_class);
    let mut matching = release
        .artifacts
        .iter()
        .filter(|artifact| artifact_required_features_supported(artifact, supported_features))
        .cloned()
        .collect::<Vec<_>>();
    matching.sort_by(|left, right| {
        artifact_feature_specificity(right)
            .cmp(&artifact_feature_specificity(left))
            .then_with(|| {
                let left_preferred =
                    artifact_encoding_from_url(&left.url) == Some(preferred_encoding);
                let right_preferred =
                    artifact_encoding_from_url(&right.url) == Some(preferred_encoding);
                right_preferred.cmp(&left_preferred)
            })
    });
    matching.into_iter().next()
}
fn select_catalog_release_and_artifact(
    plugin: &CatalogV3PluginEntry,
    supported_features: &HashSet<String>,
    cpu_class: crate::services::RuntimePerformanceClass,
) -> Option<(CatalogV3PluginRelease, CatalogV3PluginArtifact)> {
    plugin
        .releases
        .iter()
        .filter(|release| catalog_release_is_sdk_compatible(&plugin.id, release))
        .filter_map(|release| {
            select_catalog_release_artifact(release, supported_features, cpu_class)
                .map(|artifact| (release, artifact))
        })
        .filter_map(|(release, artifact)| {
            parse_catalog_release_version(&plugin.id, release)
                .map(|version| (version, release, artifact))
        })
        .max_by(|(left, _, _), (right, _, _)| left.cmp(right))
        .map(|(_, release, artifact)| (release.clone(), artifact))
}
fn parse_rule_pack_release_version(
    pack_id: &str,
    release: &CatalogV3RulePackRelease,
) -> Option<semver::Version> {
    semver::Version::parse(release.version.trim_start_matches('v')).map_or_else(
        |error| {
            warn!(
                pack_id,
                version = release.version.as_str(),
                error = %error,
                "skipping rule pack release with invalid version"
            );
            None
        },
        Some,
    )
}
fn preferred_distribution_artifact_encoding(
    cpu_class: crate::services::RuntimePerformanceClass,
) -> &'static str {
    preferred_plugin_artifact_encoding(cpu_class)
}
fn select_distribution_artifact(
    artifacts: &[CatalogV3DistributionArtifact],
    cpu_class: crate::services::RuntimePerformanceClass,
) -> Option<CatalogV3DistributionArtifact> {
    let preferred_encoding = preferred_distribution_artifact_encoding(cpu_class);
    let mut matching = artifacts.to_vec();
    matching.sort_by(|left, right| {
        let left_preferred = artifact_encoding_from_url(&left.url) == Some(preferred_encoding);
        let right_preferred = artifact_encoding_from_url(&right.url) == Some(preferred_encoding);
        right_preferred.cmp(&left_preferred)
    });
    matching.into_iter().next()
}
fn select_rule_pack_release_and_artifact(
    pack: &CatalogV3RulePackEntry,
    cpu_class: crate::services::RuntimePerformanceClass,
) -> Option<(CatalogV3RulePackRelease, CatalogV3DistributionArtifact)> {
    pack.releases
        .iter()
        .filter(|release| {
            release
                .min_scryer_version
                .as_ref()
                .and_then(|v| semver::Version::parse(v).ok())
                .is_none_or(|min| current_scryer_version() >= &min)
        })
        .filter_map(|release| {
            select_distribution_artifact(&release.artifacts, cpu_class)
                .map(|artifact| (release, artifact))
        })
        .filter_map(|(release, artifact)| {
            parse_rule_pack_release_version(&pack.id, release)
                .map(|version| (version, release, artifact))
        })
        .max_by(|(left, _, _), (right, _, _)| left.cmp(right))
        .map(|(_, release, artifact)| (release.clone(), artifact))
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
const DEFAULT_CATALOG_URL: &str = "https://cdn.scryer.media/catalog/v3/catalog-v3.redirect.json";
const FALLBACK_CATALOG_URL: &str = "https://github.com/scryer-media/scryer-plugins/releases/download/catalog%2Fv3/catalog-v3.redirect.json";
const CATALOG_URL_ENV: &str = "SCRYER_PLUGIN_CATALOG_URL";
const CENTRAL_CATALOG_SOURCE_KEY: &str = "__central_catalog";
const LEGACY_CENTRAL_CATALOG_SOURCE_KEY: &str = "__central_catalog_v2";
const CENTRAL_CATALOG_REPO: &str = "scryer-media/scryer-plugins";
const CENTRAL_CATALOG_WORKFLOW: &str = ".github/workflows/release-plugin.yml";
fn plugin_catalog_url() -> String {
    std::env::var(CATALOG_URL_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_CATALOG_URL.to_string())
}
fn fallback_plugin_catalog_url() -> &'static str {
    FALLBACK_CATALOG_URL
}
fn signed_catalog_json_bundle_url(url: &str) -> String {
    format!("{url}.bundle.zst")
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
async fn fetch_plugin_bytes_from_locations(
    locations: &[String],
    label: &str,
    scope_prefix: &str,
) -> AppResult<(Vec<u8>, String)> {
    let mut last_error = None;
    for (index, url) in locations.iter().enumerate() {
        match fetch_plugin_bytes(url, label, format!("{scope_prefix}:{index}")).await {
            Ok(bytes) => return Ok((bytes, url.clone())),
            Err(error) => {
                debug!(%url, error = %error, "plugin fetch location failed");
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        AppError::Repository(format!(
            "failed to download {label}: no candidate URLs were available"
        ))
    }))
}

async fn decode_signature_bundle(bundle: Vec<u8>, actual_url: &str) -> AppResult<Vec<u8>> {
    match artifact_encoding_from_url(actual_url) {
        Some("zst") => decompress_zstd(bundle).await,
        Some("br") => decompress_brotli(bundle).await,
        _ => Ok(bundle),
    }
}

impl AppUseCase {
    fn validate_catalog_downloaded_plugin_release(
        &self,
        plugin_id: &str,
        expected_plugin_type: &str,
        expected_provider_type: &str,
        release: &DownloadedPluginReleaseContract,
        wasm_bytes: &[u8],
    ) -> AppResult<ValidatedDownloadedPlugin> {
        let descriptor = self
            .services
            .customization
            .plugin_descriptor_loader
            .load_descriptor_from_wasm_bytes(wasm_bytes)?;
        validate_downloaded_plugin_descriptor(
            plugin_id,
            expected_plugin_type,
            expected_provider_type,
            release,
            &descriptor,
            false,
        )
    }
}
impl AppUseCase {
    async fn cached_central_catalog(&self) -> AppResult<Option<CatalogV3>> {
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
        match parse_and_validate_catalog_v3(json.as_bytes()) {
            Ok(catalog) => Ok(Some(catalog)),
            Err(error) => {
                warn!(error = %error, "cached central plugin catalog is invalid");
                Ok(None)
            }
        }
    }
}
impl AppUseCase {
    async fn load_rule_pack_catalog(&self) -> AppResult<CatalogV3> {
        if let Some(catalog) = self.cached_central_catalog().await? {
            return Ok(catalog);
        }

        self.refresh_plugin_catalog_internal().await?;
        self.cached_central_catalog().await?.ok_or_else(|| {
            AppError::Repository("central plugin catalog is unavailable".to_string())
        })
    }
}
impl AppUseCase {
    pub async fn refresh_plugin_catalog(&self, actor: &User) -> AppResult<Vec<RegistryPlugin>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.refresh_plugin_catalog_internal().await?;
        self.list_available_plugins(actor).await
    }
}
impl AppUseCase {
    pub async fn refresh_plugin_catalog_internal(&self) -> AppResult<()> {
        let (central, redirect_url, _) = self.fetch_verified_catalog_v3().await?;
        let central_json = serde_json::to_string(&central).map_err(|error| {
            AppError::Repository(format!("failed to serialize plugin catalog cache: {error}"))
        })?;
        let now = Utc::now();
        self.services
            .customization
            .plugin_installations
            .upsert_plugin_catalog_source(&PluginCatalogSource {
                source_key: CENTRAL_CATALOG_SOURCE_KEY.to_string(),
                source_kind: "central".to_string(),
                source_url: redirect_url,
                github_repo: Some(CENTRAL_CATALOG_REPO.to_string()),
                support_tier: PluginSupportTier::Official,
                catalog_json: Some(central_json),
                last_success_at: Some(now),
                last_error: None,
                updated_at: now,
            })
            .await?;

        let sources = self
            .services
            .customization
            .plugin_installations
            .list_plugin_catalog_sources()
            .await?;

        for stale_source in sources.iter().filter(|source| {
            source.source_key == LEGACY_CENTRAL_CATALOG_SOURCE_KEY || source.source_kind == "child"
        }) {
            self.services
                .customization
                .plugin_installations
                .delete_plugin_catalog_source(&stale_source.source_key)
                .await?;
        }

        for source in sources
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
                let catalog_url = if source_url.trim().is_empty() {
                    repo.catalog_v3_url()
                } else {
                    source_url.clone()
                };
                let (_, catalog_json) = self
                    .resolve_manual_plugin_repo_at_url(repo.clone(), &catalog_url)
                    .await?;
                self.upsert_manual_plugin_catalog_source(
                    &repo,
                    &catalog_url,
                    Some(catalog_json),
                    None,
                )
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
                    let catalog_url = if source_url.trim().is_empty() {
                        repo.catalog_v3_url()
                    } else {
                        source_url.clone()
                    };
                    self.upsert_manual_plugin_catalog_source(
                        &repo,
                        &catalog_url,
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
}
impl AppUseCase {
    async fn fetch_verified_blob_from_locations(
        &self,
        data_urls: &[String],
        signature_urls: &[String],
        signer: &RequiredSigner,
        label: &str,
    ) -> AppResult<(Vec<u8>, String)> {
        let scope = format!("verified_blob:{}", blake3_digest(label.as_bytes()));
        let (raw, actual_url) =
            fetch_plugin_bytes_from_locations(data_urls, label, &format!("{scope}:blob")).await?;
        let (bundle, bundle_url) = fetch_plugin_bytes_from_locations(
            signature_urls,
            &format!("{label} signature"),
            &format!("{scope}:signature"),
        )
        .await?;
        let bundle = decode_signature_bundle(bundle, &bundle_url).await?;
        verify_signed_blob(raw.clone(), bundle, signer.clone()).await?;
        Ok((raw, actual_url))
    }
}
impl AppUseCase {
    async fn fetch_verified_catalog_redirect(&self) -> AppResult<(CatalogV3Redirect, String)> {
        let signer = RequiredSigner {
            github_repository: CENTRAL_CATALOG_REPO.to_string(),
            github_workflow: Some(CENTRAL_CATALOG_WORKFLOW.to_string()),
        };
        let primary_url = plugin_catalog_url();
        let fallback_url = fallback_plugin_catalog_url().to_string();
        let candidate_urls = vec![primary_url, fallback_url];
        let mut last_error = None;
        for url in candidate_urls {
            let data_urls = vec![url.clone()];
            let signature_urls = vec![redirect_bundle_url_for(&url)];
            match self
                .fetch_verified_blob_from_locations(
                    &data_urls,
                    &signature_urls,
                    &signer,
                    "plugin catalog redirect",
                )
                .await
            {
                Ok((raw, actual_url)) => {
                    let redirect = parse_and_validate_catalog_v3_redirect(&raw)?;
                    return Ok((redirect, actual_url));
                }
                Err(error) => {
                    debug!(redirect_url = %url, error = %error, "plugin catalog redirect candidate failed");
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            AppError::Repository("plugin catalog redirect is unavailable".to_string())
        }))
    }
}
impl AppUseCase {
    async fn fetch_verified_catalog_v3(&self) -> AppResult<(CatalogV3, String, u64)> {
        let (redirect, redirect_url) = self.fetch_verified_catalog_redirect().await?;
        let signer = RequiredSigner {
            github_repository: CENTRAL_CATALOG_REPO.to_string(),
            github_workflow: Some(CENTRAL_CATALOG_WORKFLOW.to_string()),
        };
        let artifact = redirect.artifacts.first().cloned().ok_or_else(|| {
            AppError::Validation(
                "plugin catalog redirect did not contain any artifacts".to_string(),
            )
        })?;
        let data_urls = primary_and_mirrors(&artifact.url, &artifact.mirror_urls);
        let signature_urls =
            primary_and_mirrors(&artifact.signature_url, &artifact.signature_mirror_urls);
        let (raw, actual_url) = self
            .fetch_verified_blob_from_locations(
                &data_urls,
                &signature_urls,
                &signer,
                "plugin catalog",
            )
            .await?;
        let decoded = match artifact_encoding_from_url(&actual_url) {
            Some("zst") => decompress_zstd(raw).await?,
            Some("br") => decompress_brotli(raw).await?,
            _ => raw,
        };
        let catalog = parse_and_validate_catalog_v3(&decoded)?;
        Ok((catalog, redirect_url, redirect.catalog_version))
    }
}
impl AppUseCase {
    async fn resolved_catalog_plugins(&self) -> AppResult<Vec<CatalogPluginResolution>> {
        let sources = self
            .services
            .customization
            .plugin_installations
            .list_plugin_catalog_sources()
            .await?;
        let supported_plugin_features = self.runtime_supported_plugin_required_features();
        let cpu_class = self.runtime_performance().await.cpu_class;
        let central = sources
            .iter()
            .find(|source| source.source_key == CENTRAL_CATALOG_SOURCE_KEY)
            .and_then(|source| source.catalog_json.as_deref())
            .and_then(|json| parse_and_validate_catalog_v3(json.as_bytes()).ok());

        let mut result = Vec::new();
        if let Some(central) = central {
            for entry in central.plugins {
                let Some((release, artifact)) = select_catalog_release_and_artifact(
                    &entry,
                    &supported_plugin_features,
                    cpu_class,
                ) else {
                    continue;
                };
                let github_repo = GitHubRepo::parse(&entry.source_repo)?;
                result.push(CatalogPluginResolution {
                    catalog_entry: entry.clone(),
                    release,
                    artifact,
                    effective_support_tier: entry.support_tier,
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
            let (catalog_json, repo_slug) = source;
            let manual_repo = GitHubRepo::parse(repo_slug)?;
            let catalog = parse_and_validate_catalog_v3(catalog_json.as_bytes())?;
            let plugin = single_manual_catalog_plugin(&catalog, &manual_repo)?;
            let Some((release, artifact)) =
                select_catalog_release_and_artifact(&plugin, &supported_plugin_features, cpu_class)
            else {
                continue;
            };
            result.push(CatalogPluginResolution {
                catalog_entry: plugin,
                release,
                artifact,
                source_kind: PluginSourceKind::Manual,
                effective_support_tier: PluginSupportTier::Unverified,
                github_repo: manual_repo,
            });
        }

        Ok(result)
    }
}
impl AppUseCase {
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
            .any(|plugin| plugin.catalog_entry.id == plugin_id);
        if available {
            Ok(())
        } else {
            Err(AppError::NotFound(format!(
                "plugin '{plugin_id}' is not available from the plugin catalog"
            )))
        }
    }
}
impl AppUseCase {
    async fn validate_catalog_upgrade_request(&self, plugin_id: &str) -> AppResult<()> {
        self.services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not installed")))?;
        Ok(())
    }
}
impl AppUseCase {
    /// List available community rule packs from the cached central catalog.
    pub async fn list_rule_pack_registry(
        &self,
        actor: &User,
    ) -> AppResult<Vec<RulePackRegistryEntry>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let catalog = self.load_rule_pack_catalog().await?;
        let cpu_class = self.runtime_performance().await.cpu_class;
        Ok(catalog
            .rule_packs
            .into_iter()
            .filter_map(|pack| {
                select_rule_pack_release_and_artifact(&pack, cpu_class).map(
                    |(release, artifact)| {
                        RulePackRegistryEntry::from_release(&pack, &release, &artifact)
                    },
                )
            })
            .collect())
    }
}
impl AppUseCase {
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
        let catalog = self.load_rule_pack_catalog().await?;
        let cpu_class = self.runtime_performance().await.cpu_class;
        let pack_entry = catalog
            .rule_packs
            .iter()
            .find(|candidate| candidate.id == pack.id)
            .ok_or_else(|| AppError::NotFound(format!("rule pack {pack_id}")))?;
        let (release, artifact) = select_rule_pack_release_and_artifact(pack_entry, cpu_class)
            .ok_or_else(|| {
                AppError::Validation(format!("rule pack '{pack_id}' has no compatible artifact"))
            })?;
        let signer = RequiredSigner {
            github_repository: CENTRAL_CATALOG_REPO.to_string(),
            github_workflow: Some(CENTRAL_CATALOG_WORKFLOW.to_string()),
        };
        let (compressed_manifest, actual_url) = self
            .fetch_verified_blob_from_locations(
                &primary_and_mirrors(&artifact.url, &artifact.mirror_urls),
                &primary_and_mirrors(&artifact.signature_url, &artifact.signature_mirror_urls),
                &signer,
                "rule pack artifact",
            )
            .await?;
        verify_digest_set(
            "compressed rule pack artifact",
            &artifact.digests,
            &compressed_manifest,
        )?;
        let manifest_bytes = match artifact_encoding_from_url(&actual_url) {
            Some("br") => decompress_brotli(compressed_manifest).await?,
            Some("zst") => decompress_zstd(compressed_manifest).await?,
            _ => {
                return Err(AppError::Validation(format!(
                    "rule pack '{}' selected artifact '{}' has unsupported encoding",
                    pack_id, actual_url
                )));
            }
        };
        verify_digest_set(
            "rule pack manifest",
            &release.rule_pack_digests,
            &manifest_bytes,
        )?;
        let manifest: RulePackManifest = serde_json::from_slice(&manifest_bytes)
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
