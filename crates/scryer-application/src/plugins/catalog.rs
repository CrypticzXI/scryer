use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, OnceLock},
    time::Duration,
};

use base64::Engine;
use const_oid::db::rfc5280::ID_KP_CODE_SIGNING;
use rustls_pki_types::{CertificateDer, TrustAnchor, UnixTime};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sigstore::{
    cosign::{CosignCapabilities, bundle::SignedArtifactBundle},
    crypto::{CosignVerificationKey, SigningScheme},
    trust::{TrustRoot, sigstore::SigstoreTrustRoot},
};
use tokio::sync::Semaphore;
use tracing::debug;
use url::Url;
use webpki::{EndEntityCert, KeyUsage};
use x509_cert::{
    Certificate,
    der::{DecodePem, Encode},
    ext::{
        Extension,
        pkix::{SubjectAltName, name::GeneralName},
    },
};

use crate::{AppError, AppResult};
use scryer_domain::PluginSupportTier;

const CENTRAL_CATALOG_SCHEMA_VERSION: &str = "scryer.plugin.catalog.v2";
const CHILD_CATALOG_SCHEMA_VERSION: &str = "scryer.plugin.child_catalog.v2";
const RELEASE_MANIFEST_SCHEMA_VERSION: &str = "scryer.plugin.v1";
const PLUGIN_ARTIFACT_NAME: &str = "plugin.wasm.zst";
const PLUGIN_MANIFEST_NAME: &str = "plugin.manifest.json";
const ZSTD_COMPRESSION_LABEL: &str = "zstd";
const SIGSTORE_GITHUB_WORKFLOW_NAME_OID: &str = "1.3.6.1.4.1.57264.1.4";
const SIGSTORE_GITHUB_WORKFLOW_REPOSITORY_OID: &str = "1.3.6.1.4.1.57264.1.5";
const SIGSTORE_GITHUB_WORKFLOW_REF_OID: &str = "1.3.6.1.4.1.57264.1.6";
type RekorVerificationKeys = BTreeMap<String, CosignVerificationKey>;
type FulcioTrustAnchors = Vec<TrustAnchor<'static>>;

static REKOR_VERIFICATION_KEYS: OnceLock<Result<Arc<RekorVerificationKeys>, String>> =
    OnceLock::new();
static FULCIO_TRUST_ANCHORS: OnceLock<Result<Arc<FulcioTrustAnchors>, String>> = OnceLock::new();

static VERIFY_LIMIT: OnceLock<Semaphore> = OnceLock::new();

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CentralCatalog {
    pub schema_version: String,
    pub plugins: Vec<CentralCatalogEntry>,
    #[serde(default)]
    pub rule_packs: Vec<RulePackCatalogEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CentralCatalogEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub plugin_type: String,
    pub provider_type: String,
    pub publisher: String,
    pub support_tier: PluginSupportTier,
    pub docs_url: String,
    pub source_repo: String,
    pub child_catalog_url: String,
    pub required_signer: RequiredSigner,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulePackCatalogEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub version: String,
    pub url: String,
    #[serde(default)]
    pub min_scryer_version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredSigner {
    pub github_repository: String,
    #[serde(default)]
    pub github_workflow: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChildCatalog {
    pub schema_version: String,
    pub id: String,
    pub name: String,
    pub description: String,
    pub plugin_type: String,
    pub provider_type: String,
    pub publisher: String,
    pub support_tier: PluginSupportTier,
    pub docs_url: String,
    pub source_repo: String,
    pub releases: Vec<ChildCatalogRelease>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChildCatalogRelease {
    pub version: String,
    pub sdk_constraint: String,
    pub artifact_manifest_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginReleaseManifest {
    pub schema_version: String,
    pub id: String,
    pub plugin_type: String,
    pub provider_type: String,
    pub version: String,
    pub publisher: String,
    pub artifact: String,
    pub compression: String,
    pub wasm_digest: String,
    pub artifact_digest: String,
    pub signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubRepo {
    pub owner: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct CatalogOutageStatus {
    pub github_available: bool,
    pub blocked_actions: Vec<String>,
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubStatusSummary {
    status: GitHubOverallStatus,
    components: Vec<GitHubStatusComponent>,
}

#[derive(Debug, Deserialize)]
struct GitHubOverallStatus {
    indicator: String,
}

#[derive(Debug, Deserialize)]
struct GitHubStatusComponent {
    name: String,
    status: String,
}

impl GitHubRepo {
    pub fn parse(input: &str) -> AppResult<Self> {
        let trimmed = input.trim().trim_end_matches('/');
        if let Some((owner, name)) = trimmed.split_once('/')
            && !trimmed.starts_with("http://")
            && !trimmed.starts_with("https://")
        {
            return Self::from_parts(owner, name);
        }

        let url = Url::parse(trimmed)
            .map_err(|e| AppError::Validation(format!("invalid GitHub repository URL: {e}")))?;
        if url.host_str() != Some("github.com") {
            return Err(AppError::Validation(
                "manual plugin repositories must be hosted on github.com".to_string(),
            ));
        }
        let mut segments = url
            .path_segments()
            .ok_or_else(|| AppError::Validation("invalid GitHub repository URL".to_string()))?;
        let owner = segments
            .next()
            .ok_or_else(|| AppError::Validation("GitHub owner is missing".to_string()))?;
        let name = segments
            .next()
            .ok_or_else(|| AppError::Validation("GitHub repo name is missing".to_string()))?;
        Self::from_parts(owner, name)
    }

    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn release_asset_prefix(&self) -> String {
        format!(
            "https://github.com/{}/{}/releases/download/",
            self.owner, self.name
        )
    }

    pub fn child_catalog_url(&self) -> String {
        format!(
            "https://github.com/{}/{}/releases/latest/download/catalog-v2.min.json.zst",
            self.owner, self.name
        )
    }

    fn from_parts(owner: &str, name: &str) -> AppResult<Self> {
        let owner = owner.trim();
        let name = name.trim().trim_end_matches(".git");
        if owner.is_empty() || name.is_empty() {
            return Err(AppError::Validation(
                "GitHub repository must include owner and repo".to_string(),
            ));
        }
        Ok(Self {
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }
}

pub fn parse_and_validate_central_catalog(raw: &[u8]) -> AppResult<CentralCatalog> {
    let catalog: CentralCatalog = serde_json::from_slice(raw)
        .map_err(|e| AppError::Validation(format!("invalid central plugin catalog JSON: {e}")))?;
    validate_central_catalog(&catalog)?;
    Ok(catalog)
}

pub fn parse_and_validate_child_catalog(
    raw: &[u8],
    central: Option<&CentralCatalogEntry>,
    manual_repo: Option<&GitHubRepo>,
) -> AppResult<ChildCatalog> {
    let catalog: ChildCatalog = serde_json::from_slice(raw)
        .map_err(|e| AppError::Validation(format!("invalid child plugin catalog JSON: {e}")))?;
    validate_child_catalog(&catalog, central, manual_repo)?;
    Ok(catalog)
}

pub fn parse_and_validate_release_manifest(
    raw: &[u8],
    child: &ChildCatalog,
    release: &ChildCatalogRelease,
    expected_repo: &GitHubRepo,
) -> AppResult<PluginReleaseManifest> {
    let manifest: PluginReleaseManifest = serde_json::from_slice(raw)
        .map_err(|e| AppError::Validation(format!("invalid plugin release manifest JSON: {e}")))?;
    validate_release_manifest(&manifest, child, release, expected_repo)?;
    Ok(manifest)
}

pub async fn verify_signed_blob(
    raw: Vec<u8>,
    bundle_raw: Vec<u8>,
    required_signer: RequiredSigner,
) -> AppResult<()> {
    let permit = VERIFY_LIMIT
        .get_or_init(|| Semaphore::new(2))
        .acquire()
        .await
        .map_err(|_| AppError::Repository("plugin verification worker is closed".to_string()))?;
    let result = tokio::task::spawn_blocking(move || {
        verify_signed_blob_blocking(&raw, &bundle_raw, &required_signer)
    })
    .await
    .map_err(|e| AppError::Repository(format!("plugin signature verification panicked: {e}")))?;
    drop(permit);
    result
}

pub async fn decompress_zstd(compressed: Vec<u8>) -> AppResult<Vec<u8>> {
    tokio::task::spawn_blocking(move || {
        zstd::decode_all(compressed.as_slice())
            .map_err(|e| AppError::Repository(format!("failed to decompress zstd payload: {e}")))
    })
    .await
    .map_err(|e| AppError::Repository(format!("zstd decompression panicked: {e}")))?
}

pub async fn compress_zstd(bytes: Vec<u8>, level: i32) -> AppResult<Vec<u8>> {
    tokio::task::spawn_blocking(move || {
        zstd::encode_all(bytes.as_slice(), level)
            .map_err(|e| AppError::Repository(format!("failed to compress zstd payload: {e}")))
    })
    .await
    .map_err(|e| AppError::Repository(format!("zstd compression panicked: {e}")))?
}

pub fn blake3_digest(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

pub fn parse_digest_string(input: &str) -> AppResult<(String, String)> {
    let trimmed = input.trim();
    let (algo, digest) = trimmed
        .split_once(':')
        .ok_or_else(|| AppError::Validation(format!("invalid digest string '{trimmed}'")))?;
    let algo = algo.trim().to_ascii_lowercase();
    let digest = normalize_hex_digest(digest)?;
    if algo.is_empty() {
        return Err(AppError::Validation(
            "digest algorithm is missing".to_string(),
        ));
    }
    Ok((algo, digest))
}

pub fn verify_split_digest(
    label: &str,
    algorithm: &str,
    expected_digest: &str,
    bytes: &[u8],
) -> AppResult<()> {
    let normalized_algorithm = algorithm.trim().to_ascii_lowercase();
    let expected_digest = normalize_hex_digest(expected_digest)?;
    match normalized_algorithm.as_str() {
        "blake3" => {
            let actual_digest = blake3::hash(bytes).to_hex().to_string();
            if actual_digest.eq_ignore_ascii_case(&expected_digest) {
                Ok(())
            } else {
                Err(AppError::Validation(format!(
                    "{label} digest mismatch: expected blake3:{expected_digest}, got blake3:{actual_digest}"
                )))
            }
        }
        _ => Err(AppError::Validation(format!(
            "{label} uses unsupported digest algorithm '{normalized_algorithm}'"
        ))),
    }
}

pub fn verify_digest(label: &str, expected: &str, bytes: &[u8]) -> AppResult<()> {
    let (algorithm, digest) = parse_digest_string(expected)?;
    verify_split_digest(label, &algorithm, &digest, bytes)
}

pub fn github_outage_status_from_summary(raw: &[u8]) -> Option<CatalogOutageStatus> {
    let summary: GitHubStatusSummary = serde_json::from_slice(raw).ok()?;
    if summary.status.indicator == "none" {
        return Some(CatalogOutageStatus {
            github_available: true,
            blocked_actions: Vec::new(),
            message: None,
        });
    }

    let relevant_outage = summary.components.iter().any(|component| {
        matches!(component.name.as_str(), "API Requests" | "Git Operations")
            && component.status != "operational"
    });
    if !relevant_outage {
        return Some(CatalogOutageStatus {
            github_available: true,
            blocked_actions: Vec::new(),
            message: None,
        });
    }

    Some(CatalogOutageStatus {
        github_available: false,
        blocked_actions: vec![
            "catalog_refresh".to_string(),
            "install".to_string(),
            "install_manual".to_string(),
            "upgrade".to_string(),
            "manual_repo_inspection".to_string(),
        ],
        message: Some(
            "GitHub is reporting an outage that affects plugin distribution.".to_string(),
        ),
    })
}

fn validate_central_catalog(catalog: &CentralCatalog) -> AppResult<()> {
    if catalog.schema_version != CENTRAL_CATALOG_SCHEMA_VERSION {
        return Err(AppError::Validation(format!(
            "unsupported central plugin catalog schema '{}'",
            catalog.schema_version
        )));
    }

    let mut plugin_ids = HashSet::new();
    for entry in &catalog.plugins {
        require_non_empty("plugin id", &entry.id)?;
        require_non_empty("plugin name", &entry.name)?;
        require_non_empty("plugin type", &entry.plugin_type)?;
        require_non_empty("provider type", &entry.provider_type)?;
        require_non_empty("publisher", &entry.publisher)?;
        require_non_empty("docs_url", &entry.docs_url)?;
        require_non_empty("source_repo", &entry.source_repo)?;
        require_non_empty("child_catalog_url", &entry.child_catalog_url)?;
        if !plugin_ids.insert(entry.id.clone()) {
            return Err(AppError::Validation(format!(
                "duplicate plugin id '{}' in central catalog",
                entry.id
            )));
        }
        if entry.support_tier == PluginSupportTier::Unverified {
            return Err(AppError::Validation(format!(
                "central catalog entry '{}' cannot declare unverified support",
                entry.id
            )));
        }
        let source_repo = GitHubRepo::parse(&entry.source_repo)?;
        if entry.required_signer.github_repository != source_repo.slug() {
            return Err(AppError::Validation(format!(
                "central catalog entry '{}' signer repo '{}' does not match source repo '{}'",
                entry.id,
                entry.required_signer.github_repository,
                source_repo.slug()
            )));
        }
        require_release_asset_url("child catalog", &entry.child_catalog_url, &source_repo)?;
    }

    let catalog_repo = GitHubRepo::parse("https://github.com/scryer-media/scryer-plugins")?;
    let mut rule_pack_ids = HashSet::new();
    for entry in &catalog.rule_packs {
        require_non_empty("rule pack id", &entry.id)?;
        require_non_empty("rule pack name", &entry.name)?;
        require_non_empty("rule pack author", &entry.author)?;
        require_non_empty("rule pack version", &entry.version)?;
        require_non_empty("rule pack url", &entry.url)?;
        if !rule_pack_ids.insert(entry.id.clone()) {
            return Err(AppError::Validation(format!(
                "duplicate rule pack id '{}' in central catalog",
                entry.id
            )));
        }
        semver::Version::parse(entry.version.trim()).map_err(|error| {
            AppError::Validation(format!(
                "rule pack '{}' has invalid version '{}': {error}",
                entry.id, entry.version
            ))
        })?;
        if let Some(min_scryer_version) = entry.min_scryer_version.as_deref() {
            semver::Version::parse(min_scryer_version.trim()).map_err(|error| {
                AppError::Validation(format!(
                    "rule pack '{}' has invalid min_scryer_version '{}': {error}",
                    entry.id, min_scryer_version
                ))
            })?;
        }
        require_release_asset_url("rule pack", &entry.url, &catalog_repo)?;
    }

    Ok(())
}

fn validate_child_catalog(
    catalog: &ChildCatalog,
    central: Option<&CentralCatalogEntry>,
    manual_repo: Option<&GitHubRepo>,
) -> AppResult<()> {
    if catalog.schema_version != CHILD_CATALOG_SCHEMA_VERSION {
        return Err(AppError::Validation(format!(
            "unsupported child plugin catalog schema '{}'",
            catalog.schema_version
        )));
    }

    require_non_empty("plugin id", &catalog.id)?;
    require_non_empty("plugin name", &catalog.name)?;
    require_non_empty("plugin type", &catalog.plugin_type)?;
    require_non_empty("provider type", &catalog.provider_type)?;
    require_non_empty("publisher", &catalog.publisher)?;
    require_non_empty("docs_url", &catalog.docs_url)?;
    require_non_empty("source_repo", &catalog.source_repo)?;
    let source_repo = GitHubRepo::parse(&catalog.source_repo)?;

    if let Some(central) = central {
        require_identity_match("id", &central.id, &catalog.id)?;
        require_identity_match("plugin_type", &central.plugin_type, &catalog.plugin_type)?;
        require_identity_match(
            "provider_type",
            &central.provider_type,
            &catalog.provider_type,
        )?;
        require_identity_match("publisher", &central.publisher, &catalog.publisher)?;
        if central.support_tier != catalog.support_tier {
            return Err(AppError::Validation(format!(
                "child catalog '{}' support tier does not match central catalog",
                catalog.id
            )));
        }
        let central_repo = GitHubRepo::parse(&central.source_repo)?;
        if source_repo != central_repo {
            return Err(AppError::Validation(format!(
                "child catalog '{}' source repo does not match central catalog",
                catalog.id
            )));
        }
    }

    if let Some(manual_repo) = manual_repo
        && &source_repo != manual_repo
    {
        return Err(AppError::Validation(format!(
            "manual child catalog source repo '{}' does not match requested repo '{}'",
            source_repo.slug(),
            manual_repo.slug()
        )));
    }

    let mut versions = HashSet::new();
    for release in &catalog.releases {
        parse_version(&release.version)?;
        VersionReq::parse(&release.sdk_constraint).map_err(|e| {
            AppError::Validation(format!(
                "invalid sdk_constraint '{}' for plugin '{}': {e}",
                release.sdk_constraint, catalog.id
            ))
        })?;
        if !versions.insert(release.version.clone()) {
            return Err(AppError::Validation(format!(
                "duplicate release version '{}' for plugin '{}'",
                release.version, catalog.id
            )));
        }
        require_release_asset_url(
            "release manifest",
            &release.artifact_manifest_url,
            &source_repo,
        )?;
    }

    Ok(())
}

fn validate_release_manifest(
    manifest: &PluginReleaseManifest,
    child: &ChildCatalog,
    release: &ChildCatalogRelease,
    expected_repo: &GitHubRepo,
) -> AppResult<()> {
    if manifest.schema_version != RELEASE_MANIFEST_SCHEMA_VERSION {
        return Err(AppError::Validation(format!(
            "unsupported plugin release manifest schema '{}'",
            manifest.schema_version
        )));
    }

    require_identity_match("id", &child.id, &manifest.id)?;
    require_identity_match("plugin_type", &child.plugin_type, &manifest.plugin_type)?;
    require_identity_match(
        "provider_type",
        &child.provider_type,
        &manifest.provider_type,
    )?;
    require_identity_match("publisher", &child.publisher, &manifest.publisher)?;
    require_identity_match("version", &release.version, &manifest.version)?;
    if manifest.artifact != PLUGIN_ARTIFACT_NAME {
        return Err(AppError::Validation(format!(
            "plugin manifest '{}' uses unsupported artifact '{}'",
            manifest.id, manifest.artifact
        )));
    }
    if manifest.compression != ZSTD_COMPRESSION_LABEL {
        return Err(AppError::Validation(format!(
            "plugin manifest '{}' uses unsupported compression '{}'",
            manifest.id, manifest.compression
        )));
    }
    require_digest("wasm_digest", &manifest.wasm_digest)?;
    require_digest("artifact_digest", &manifest.artifact_digest)?;
    if manifest.signature != format!("{PLUGIN_ARTIFACT_NAME}.bundle") {
        return Err(AppError::Validation(format!(
            "plugin manifest '{}' signature must be '{}.bundle'",
            manifest.id, PLUGIN_ARTIFACT_NAME
        )));
    }
    require_release_asset_url(
        "plugin manifest",
        &release.artifact_manifest_url,
        expected_repo,
    )?;
    Ok(())
}

fn verify_signed_blob_blocking(
    raw: &[u8],
    bundle_raw: &[u8],
    required_signer: &RequiredSigner,
) -> AppResult<()> {
    let bundle_text = std::str::from_utf8(bundle_raw)
        .map_err(|e| AppError::Validation(format!("invalid Sigstore bundle UTF-8: {e}")))?;
    let bundle_text = normalize_sigstore_bundle(bundle_text)?;
    let rekor_keys = cached_rekor_verification_keys()?;

    let bundle = SignedArtifactBundle::new_verified(bundle_text.as_str(), rekor_keys.as_ref())
        .map_err(|e| {
            AppError::Validation(format!("Sigstore Rekor bundle verification failed: {e}"))
        })?;
    let cert_pem = normalize_bundle_cert(&bundle.cert)?;
    <sigstore::cosign::Client as CosignCapabilities>::verify_blob(
        &cert_pem,
        &bundle.base64_signature,
        raw,
    )
    .map_err(|e| {
        AppError::Validation(format!("Sigstore blob signature verification failed: {e}"))
    })?;
    verify_fulcio_certificate_chain(&cert_pem, &bundle)?;
    verify_signer_identity(&cert_pem, required_signer)?;
    Ok(())
}

fn verify_fulcio_certificate_chain(cert_pem: &str, bundle: &SignedArtifactBundle) -> AppResult<()> {
    let cert = Certificate::from_pem(cert_pem.as_bytes())
        .map_err(|e| AppError::Validation(format!("failed to parse Sigstore certificate: {e}")))?;
    let cert_der = cert
        .to_der()
        .map_err(|e| AppError::Validation(format!("failed to encode Sigstore certificate: {e}")))?;
    let cert_der = CertificateDer::from(cert_der.as_slice());
    let end_entity = EndEntityCert::try_from(&cert_der)
        .map_err(|e| AppError::Validation(format!("invalid Sigstore certificate: {e}")))?;
    let verification_time = rekor_integrated_time(bundle.rekor_bundle.payload.integrated_time)?;
    let trust_anchors = cached_fulcio_trust_anchors()?;

    end_entity
        .verify_for_usage(
            webpki::ALL_VERIFICATION_ALGS,
            trust_anchors.as_slice(),
            &[],
            verification_time,
            KeyUsage::required(ID_KP_CODE_SIGNING.as_bytes()),
            None,
            None,
        )
        .map_err(|e| {
            AppError::Validation(format!(
                "Sigstore Fulcio certificate chain verification failed: {e}"
            ))
        })?;

    Ok(())
}

fn rekor_integrated_time(integrated_time: i64) -> AppResult<UnixTime> {
    let integrated_time = u64::try_from(integrated_time)
        .map_err(|_| AppError::Validation("Sigstore Rekor integrated time is negative".into()))?;
    Ok(UnixTime::since_unix_epoch(Duration::from_secs(
        integrated_time,
    )))
}

fn cached_rekor_verification_keys() -> AppResult<Arc<RekorVerificationKeys>> {
    REKOR_VERIFICATION_KEYS
        .get_or_init(|| {
            let trust_root = tokio::runtime::Handle::current()
                .block_on(SigstoreTrustRoot::new(None))
                .map_err(|e| format!("failed to load Sigstore trust root: {e}"))?;
            let rekor_keys = trust_root
                .rekor_keys()
                .map_err(|e| format!("failed to load Sigstore Rekor public keys: {e}"))?;
            parse_rekor_verification_keys(rekor_keys)
                .map(Arc::new)
                .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(AppError::Repository)
}

fn cached_fulcio_trust_anchors() -> AppResult<Arc<FulcioTrustAnchors>> {
    FULCIO_TRUST_ANCHORS
        .get_or_init(|| {
            let trust_root = tokio::runtime::Handle::current()
                .block_on(SigstoreTrustRoot::new(None))
                .map_err(|e| format!("failed to load Sigstore trust root: {e}"))?;
            let fulcio_certs = trust_root
                .fulcio_certs()
                .map_err(|e| format!("failed to load Sigstore Fulcio certificates: {e}"))?;
            let anchors = fulcio_certs
                .iter()
                .map(|cert| {
                    webpki::anchor_from_trusted_cert(cert)
                        .map(|anchor| anchor.to_owned())
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            if anchors.is_empty() {
                return Err("Sigstore Fulcio trust root is empty".to_string());
            }
            Ok(Arc::new(anchors))
        })
        .clone()
        .map_err(AppError::Repository)
}

fn parse_rekor_verification_keys(
    keys: std::collections::BTreeMap<String, &[u8]>,
) -> AppResult<RekorVerificationKeys> {
    let parsed = keys
        .into_iter()
        .filter_map(|(key_id, key)| {
            match CosignVerificationKey::from_der(key, &SigningScheme::default()) {
                Ok(key) => Some((key_id, key)),
                Err(error) => {
                    debug!(%key_id, %error, "skipping unsupported Rekor public key");
                    None
                }
            }
        })
        .collect::<BTreeMap<_, _>>();
    if parsed.is_empty() {
        return Err(AppError::Repository(
            "failed to parse any Rekor public keys from the Sigstore trust root".to_string(),
        ));
    }
    Ok(parsed)
}

fn verify_signer_identity(cert_pem: &str, required_signer: &RequiredSigner) -> AppResult<()> {
    let cert = Certificate::from_pem(cert_pem.as_bytes())
        .map_err(|e| AppError::Validation(format!("failed to parse Sigstore certificate: {e}")))?;
    let repository = cert_extension_utf8(&cert, SIGSTORE_GITHUB_WORKFLOW_REPOSITORY_OID)?;
    if repository.as_deref() != Some(required_signer.github_repository.as_str()) {
        return Err(AppError::Validation(format!(
            "Sigstore signer repo mismatch: expected '{}', got '{}'",
            required_signer.github_repository,
            repository.unwrap_or_else(|| "<missing>".to_string())
        )));
    }

    if let Some(expected_workflow) = required_signer.github_workflow.as_deref() {
        let workflow_name = cert_extension_utf8(&cert, SIGSTORE_GITHUB_WORKFLOW_NAME_OID)?;
        let workflow_ref = cert_extension_utf8(&cert, SIGSTORE_GITHUB_WORKFLOW_REF_OID)?;
        let subject_uri = cert_subject_uri(&cert)?;
        let matched = workflow_name.as_deref() == Some(expected_workflow)
            || workflow_ref
                .as_deref()
                .is_some_and(|value| value.contains(expected_workflow))
            || subject_uri
                .as_deref()
                .is_some_and(|value| value.contains(expected_workflow));
        if !matched {
            return Err(AppError::Validation(format!(
                "Sigstore workflow mismatch for '{}'",
                required_signer.github_repository
            )));
        }
    }

    Ok(())
}

fn normalize_sigstore_bundle(bundle_text: &str) -> AppResult<String> {
    let Ok(bundle_json) = serde_json::from_str::<serde_json::Value>(bundle_text) else {
        return Ok(bundle_text.to_string());
    };
    if bundle_json.get("base64Signature").is_some() || bundle_json.get("messageSignature").is_none()
    {
        return Ok(bundle_text.to_string());
    }

    let tlog_entry = sigstore_bundle_value(&bundle_json, &["verificationMaterial", "tlogEntries"])
        .and_then(|value| value.as_array())
        .and_then(|entries| entries.first())
        .ok_or_else(|| {
            AppError::Validation(
                "Sigstore bundle missing verificationMaterial.tlogEntries[0]".to_string(),
            )
        })?;
    let cert_pem = normalize_bundle_cert(sigstore_bundle_string_field(
        &bundle_json,
        &["verificationMaterial", "certificate", "rawBytes"],
        "verificationMaterial.certificate.rawBytes",
    )?)?;

    serde_json::to_string(&serde_json::json!({
        "base64Signature": sigstore_bundle_string_field(
            &bundle_json,
            &["messageSignature", "signature"],
            "messageSignature.signature",
        )?,
        "cert": cert_pem,
        "rekorBundle": {
            "SignedEntryTimestamp": sigstore_bundle_string_field(
                tlog_entry,
                &["inclusionPromise", "signedEntryTimestamp"],
                "verificationMaterial.tlogEntries[0].inclusionPromise.signedEntryTimestamp",
            )?,
            "Payload": {
                "body": sigstore_bundle_string_field(
                    tlog_entry,
                    &["canonicalizedBody"],
                    "verificationMaterial.tlogEntries[0].canonicalizedBody",
                )?,
                "integratedTime": sigstore_bundle_i64_field(
                    tlog_entry,
                    &["integratedTime"],
                    "verificationMaterial.tlogEntries[0].integratedTime",
                )?,
                "logIndex": sigstore_bundle_i64_field(
                    tlog_entry,
                    &["logIndex"],
                    "verificationMaterial.tlogEntries[0].logIndex",
                )?,
                "logID": sigstore_bundle_string_field(
                    tlog_entry,
                    &["logId", "keyId"],
                    "verificationMaterial.tlogEntries[0].logId.keyId",
                )
                .map(normalize_rekor_log_id)?,
            }
        }
    }))
    .map_err(|e| AppError::Validation(format!("failed to normalize Sigstore bundle: {e}")))
}

fn sigstore_bundle_value<'a>(
    value: &'a serde_json::Value,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    path.iter()
        .try_fold(value, |current, segment| current.get(*segment))
}

fn sigstore_bundle_string_field<'a>(
    value: &'a serde_json::Value,
    path: &[&str],
    label: &str,
) -> AppResult<&'a str> {
    sigstore_bundle_value(value, path)
        .and_then(|value| value.as_str())
        .ok_or_else(|| AppError::Validation(format!("Sigstore bundle missing {label}")))
}

fn sigstore_bundle_i64_field(
    value: &serde_json::Value,
    path: &[&str],
    label: &str,
) -> AppResult<i64> {
    let Some(value) = sigstore_bundle_value(value, path) else {
        return Err(AppError::Validation(format!(
            "Sigstore bundle missing {label}"
        )));
    };
    if let Some(number) = value.as_i64() {
        return Ok(number);
    }
    let Some(number) = value.as_str() else {
        return Err(AppError::Validation(format!(
            "Sigstore bundle {label} is not an integer"
        )));
    };
    number.parse::<i64>().map_err(|e| {
        AppError::Validation(format!(
            "Sigstore bundle {label} is not a valid integer: {e}"
        ))
    })
}

fn normalize_rekor_log_id(key_id: &str) -> String {
    if key_id.len().is_multiple_of(2) && key_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return key_id.to_ascii_lowercase();
    }

    match base64::engine::general_purpose::STANDARD.decode(key_id.as_bytes()) {
        Ok(decoded) => {
            use std::fmt::Write as _;

            let mut hex = String::with_capacity(decoded.len() * 2);
            for byte in decoded {
                let _ = write!(&mut hex, "{byte:02x}");
            }
            hex
        }
        Err(_) => key_id.to_string(),
    }
}

fn normalize_bundle_cert(cert: &str) -> AppResult<String> {
    if cert.contains("-----BEGIN CERTIFICATE-----") {
        return Ok(cert.to_string());
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(cert.as_bytes())
        .map_err(|e| AppError::Validation(format!("invalid base64 Sigstore certificate: {e}")))?;
    if let Ok(decoded_text) = String::from_utf8(decoded.clone())
        && decoded_text.contains("-----BEGIN CERTIFICATE-----")
    {
        return Ok(decoded_text);
    }
    Ok(pem_encode_certificate(&decoded))
}

fn pem_encode_certificate(der: &[u8]) -> String {
    let base64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in base64.as_bytes().chunks(64) {
        pem.push_str(&String::from_utf8_lossy(chunk));
        pem.push('\n');
    }
    pem.push_str("-----END CERTIFICATE-----\n");
    pem
}

fn cert_extension_utf8(cert: &Certificate, oid: &str) -> AppResult<Option<String>> {
    let Some(extensions) = cert.tbs_certificate.extensions.as_ref() else {
        return Ok(None);
    };
    extensions
        .iter()
        .find(|ext: &&Extension| ext.extn_id.to_string() == oid)
        .map(|ext| {
            String::from_utf8(ext.extn_value.clone().into_bytes()).map_err(|_| {
                AppError::Validation(format!(
                    "Sigstore certificate extension {oid} is not valid UTF-8"
                ))
            })
        })
        .transpose()
}

fn cert_subject_uri(cert: &Certificate) -> AppResult<Option<String>> {
    let san = cert
        .tbs_certificate
        .get::<SubjectAltName>()
        .map_err(|e| AppError::Validation(format!("failed to read certificate SAN: {e}")))?
        .map(|(_, san)| san);
    let Some(san) = san else {
        return Ok(None);
    };
    Ok(san.0.iter().find_map(|name| match name {
        GeneralName::UniformResourceIdentifier(uri) => Some(uri.to_string()),
        _ => None,
    }))
}

fn require_non_empty(label: &str, value: &str) -> AppResult<()> {
    if value.trim().is_empty() {
        return Err(AppError::Validation(format!("{label} is required")));
    }
    Ok(())
}

fn require_identity_match(label: &str, expected: &str, actual: &str) -> AppResult<()> {
    if expected != actual {
        return Err(AppError::Validation(format!(
            "{label} mismatch: expected '{expected}', got '{actual}'"
        )));
    }
    Ok(())
}

fn parse_version(version: &str) -> AppResult<Version> {
    Version::parse(version.trim_start_matches('v')).map_err(|e| {
        AppError::Validation(format!("invalid plugin release version '{version}': {e}"))
    })
}

fn require_digest(label: &str, digest: &str) -> AppResult<()> {
    parse_digest_string(digest).map(|_| ()).map_err(|_| {
        AppError::Validation(format!(
            "{label} must use a supported <algorithm>:<hex> digest"
        ))
    })
}

fn normalize_hex_digest(input: &str) -> AppResult<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation("digest value is missing".to_string()));
    }
    if !trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(AppError::Validation(format!(
            "digest value '{trimmed}' must be hexadecimal"
        )));
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn require_release_asset_url(label: &str, url: &str, repo: &GitHubRepo) -> AppResult<()> {
    if !url.starts_with(&repo.release_asset_prefix()) {
        return Err(AppError::Validation(format!(
            "{label} URL must point to GitHub Releases for '{}'",
            repo.slug()
        )));
    }
    Ok(())
}

pub fn plugin_manifest_asset_url(manifest_url: &str, asset_name: &str) -> AppResult<String> {
    let parsed = Url::parse(manifest_url)
        .map_err(|e| AppError::Validation(format!("invalid plugin manifest URL: {e}")))?;
    let mut segments = parsed
        .path_segments()
        .ok_or_else(|| AppError::Validation("invalid plugin manifest URL path".to_string()))?
        .collect::<Vec<_>>();
    if segments.last().copied() != Some(PLUGIN_MANIFEST_NAME) {
        return Err(AppError::Validation(format!(
            "plugin manifest URL must end with {PLUGIN_MANIFEST_NAME}"
        )));
    }
    segments.pop();
    segments.push(asset_name);
    let mut url = parsed.clone();
    url.set_path(&segments.join("/"));
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_digest_string_splits_blake3_digest() {
        let (algorithm, digest) =
            parse_digest_string("blake3:0123456789abcdef").expect("digest should parse");
        assert_eq!(algorithm, "blake3");
        assert_eq!(digest, "0123456789abcdef");
    }

    #[test]
    fn parse_digest_string_rejects_malformed_values() {
        assert!(parse_digest_string("blake3").is_err());
        assert!(parse_digest_string("blake3:not-hex").is_err());
        assert!(parse_digest_string(":abcd").is_err());
    }

    #[test]
    fn verify_split_digest_accepts_matching_blake3_hex() {
        let bytes = b"hello from scryer";
        let digest = blake3::hash(bytes).to_hex().to_string();
        verify_split_digest("plugin wasm", "blake3", &digest, bytes)
            .expect("matching digest should verify");
    }

    #[test]
    fn verify_split_digest_rejects_unknown_algorithms() {
        let err = verify_split_digest("plugin wasm", "sha256", "abcd", b"bytes").unwrap_err();
        assert!(err.to_string().contains("unsupported digest algorithm"));
    }

    #[test]
    fn github_status_fail_open_for_malformed_response() {
        assert!(github_outage_status_from_summary(b"not json").is_none());
    }

    #[test]
    fn github_status_blocks_only_relevant_confirmed_outage() {
        let raw = br#"{
            "status": { "indicator": "major" },
            "components": [
                { "name": "API Requests", "status": "degraded_performance" },
                { "name": "Pages", "status": "operational" }
            ]
        }"#;
        let status = github_outage_status_from_summary(raw).expect("well-formed status");
        assert!(!status.github_available);
        assert!(status.blocked_actions.contains(&"install".to_string()));
    }

    #[test]
    fn child_catalog_rejects_duplicate_release_versions() {
        let raw = br#"{
            "schema_version": "scryer.plugin.child_catalog.v2",
            "id": "email",
            "name": "Email",
            "description": "Email notifications",
            "plugin_type": "notification",
            "provider_type": "email",
            "publisher": "scryer",
            "support_tier": "official",
            "docs_url": "https://github.com/scryer-media/scryer-plugins",
            "source_repo": "https://github.com/scryer-media/scryer-plugin-email",
            "releases": [
                {
                    "version": "0.1.0",
                    "sdk_constraint": "^0.13",
                    "artifact_manifest_url": "https://github.com/scryer-media/scryer-plugin-email/releases/download/v0.1.0/plugin.manifest.json"
                },
                {
                    "version": "0.1.0",
                    "sdk_constraint": "^0.13",
                    "artifact_manifest_url": "https://github.com/scryer-media/scryer-plugin-email/releases/download/v0.1.0/plugin.manifest.json"
                }
            ]
        }"#;
        let err = parse_and_validate_child_catalog(raw, None, None).unwrap_err();
        assert!(err.to_string().contains("duplicate release version"));
    }

    #[test]
    fn normalize_bundle_cert_wraps_base64_der_as_pem() {
        let der_base64 =
            base64::engine::general_purpose::STANDARD.encode([0x30, 0x03, 0x02, 0x01, 0x05]);
        let pem = normalize_bundle_cert(&der_base64).expect("DER certificate should normalize");
        assert!(pem.starts_with("-----BEGIN CERTIFICATE-----\n"));
        assert!(pem.contains(&der_base64));
        assert!(pem.ends_with("-----END CERTIFICATE-----\n"));
    }

    #[test]
    fn normalize_sigstore_bundle_rewrites_v03_payloads() {
        let der_base64 = base64::engine::general_purpose::STANDARD.encode([1_u8, 2, 3, 4]);
        let key_id_base64 = base64::engine::general_purpose::STANDARD.encode([0_u8, 1, 2, 3]);
        let bundle = serde_json::json!({
            "mediaType": "application/vnd.dev.sigstore.bundle.v0.3+json",
            "messageSignature": {
                "signature": "sig=="
            },
            "verificationMaterial": {
                "certificate": {
                    "rawBytes": der_base64
                },
                "tlogEntries": [
                    {
                        "logIndex": "12",
                        "logId": {
                            "keyId": key_id_base64
                        },
                        "integratedTime": "34",
                        "inclusionPromise": {
                            "signedEntryTimestamp": "set=="
                        },
                        "canonicalizedBody": "body=="
                    }
                ]
            }
        });

        let normalized =
            normalize_sigstore_bundle(&bundle.to_string()).expect("bundle should normalize");
        let parsed: SignedArtifactBundle =
            serde_json::from_str(&normalized).expect("bundle should parse in legacy shape");
        assert_eq!(parsed.base64_signature, "sig==");
        assert_eq!(
            parsed.cert.lines().next(),
            Some("-----BEGIN CERTIFICATE-----")
        );
        assert_eq!(parsed.rekor_bundle.payload.log_index, 12);
        assert_eq!(parsed.rekor_bundle.payload.integrated_time, 34);
        assert_eq!(parsed.rekor_bundle.payload.log_id, "00010203");
        assert_eq!(parsed.rekor_bundle.payload.body, "body==");
    }

    #[tokio::test]
    async fn sigstore_trust_root_rekor_keys_parse_as_der() {
        let trust_root = SigstoreTrustRoot::new(None)
            .await
            .expect("embedded Sigstore trust root should load");
        let rekor_keys = trust_root
            .rekor_keys()
            .expect("Sigstore trust root should provide Rekor keys");
        assert!(!rekor_keys.is_empty(), "expected at least one Rekor key");
        let parsed = parse_rekor_verification_keys(rekor_keys)
            .expect("embedded Rekor keys should parse as DER verification keys");
        assert!(!parsed.is_empty(), "expected at least one parsed Rekor key");
    }
}
