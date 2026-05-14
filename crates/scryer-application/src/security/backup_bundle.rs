use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::{AppError, AppResult};

pub const BACKUP_FORMAT_VERSION: &str = "scryer-backup-bundle-v1";
pub const BACKUP_PLAINTEXT_EXTENSION: &str = ".scryer-backup.tar.zst";
pub const BACKUP_ENCRYPTED_EXTENSION: &str = ".scryer-backup.enc";

const INSTANCE_SECRETS_FILENAME: &str = "instance-secrets.json";
const MANIFEST_FILENAME: &str = "manifest.json";
const TABLES_DIRNAME: &str = "tables";
pub const BLOB_MARKER_TYPE: &str = "__scryer_type";
pub const BLOB_MARKER_BASE64: &str = "base64";
pub const EXPORT_BATCH_SIZE: i64 = 1_000;
const ENCRYPTED_BUNDLE_MAGIC: [u8; 8] = [0x53, 0x42, 0x45, 0x5f, 0x96, 0x31, 0xc4, 0x2a];
const BACKUP_ENCRYPTION_VERSION_1: u8 = 1;
const BACKUP_ENCRYPTION_CHUNK_SIZE: usize = 1024 * 1024;
const BACKUP_ENCRYPTION_TAG_LEN: usize = 16;
const BACKUP_ENCRYPTION_MAX_CIPHERTEXT_CHUNK_LEN: usize =
    BACKUP_ENCRYPTION_CHUNK_SIZE + BACKUP_ENCRYPTION_TAG_LEN;
const BACKUP_ENCRYPTION_SALT_LEN: usize = 16;
const BACKUP_ENCRYPTION_NONCE_PREFIX_LEN: usize = 4;
const BACKUP_ENCRYPTION_METADATA_V1_LEN: usize =
    BACKUP_ENCRYPTION_SALT_LEN + BACKUP_ENCRYPTION_NONCE_PREFIX_LEN;
const BACKUP_ENCRYPTION_KEY_LEN: usize = 32;
const BACKUP_ENCRYPTION_ARGON2_M_COST_KIB: u32 = 65_536;
const BACKUP_ENCRYPTION_ARGON2_T_COST: u32 = 3;
const BACKUP_ENCRYPTION_ARGON2_P_COST: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupTableClassification {
    Export,
    Rebuild,
    Ignore,
}

#[derive(Clone, Copy, Debug)]
pub struct BackupTableCatalogEntry {
    pub table: &'static str,
    pub classification: BackupTableClassification,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BackupEncryptionMetadataV1 {
    salt: [u8; BACKUP_ENCRYPTION_SALT_LEN],
    nonce_prefix: [u8; BACKUP_ENCRYPTION_NONCE_PREFIX_LEN],
}

impl BackupEncryptionMetadataV1 {
    fn generate() -> AppResult<Self> {
        let rng = SystemRandom::new();
        let mut salt = [0_u8; BACKUP_ENCRYPTION_SALT_LEN];
        let mut nonce_prefix = [0_u8; BACKUP_ENCRYPTION_NONCE_PREFIX_LEN];
        rng.fill(&mut salt).map_err(|error| {
            AppError::Repository(format!(
                "failed to generate backup encryption salt: {error}"
            ))
        })?;
        rng.fill(&mut nonce_prefix).map_err(|error| {
            AppError::Repository(format!(
                "failed to generate backup encryption nonce prefix: {error}"
            ))
        })?;
        Ok(Self { salt, nonce_prefix })
    }

    fn to_bytes(self) -> [u8; BACKUP_ENCRYPTION_METADATA_V1_LEN] {
        let mut bytes = [0_u8; BACKUP_ENCRYPTION_METADATA_V1_LEN];
        bytes[..BACKUP_ENCRYPTION_SALT_LEN].copy_from_slice(&self.salt);
        bytes[BACKUP_ENCRYPTION_SALT_LEN..].copy_from_slice(&self.nonce_prefix);
        bytes
    }

    fn from_bytes(bytes: &[u8]) -> AppResult<Self> {
        if bytes.len() != BACKUP_ENCRYPTION_METADATA_V1_LEN {
            return Err(AppError::Validation(
                "backup encryption metadata is invalid".into(),
            ));
        }

        let mut salt = [0_u8; BACKUP_ENCRYPTION_SALT_LEN];
        salt.copy_from_slice(&bytes[..BACKUP_ENCRYPTION_SALT_LEN]);
        let mut nonce_prefix = [0_u8; BACKUP_ENCRYPTION_NONCE_PREFIX_LEN];
        nonce_prefix.copy_from_slice(&bytes[BACKUP_ENCRYPTION_SALT_LEN..]);
        Ok(Self { salt, nonce_prefix })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EncryptedBundleHeaderV1 {
    metadata: BackupEncryptionMetadataV1,
}

pub const BACKUP_TABLE_CATALOG: &[BackupTableCatalogEntry] = &[
    BackupTableCatalogEntry {
        table: "_sqlx_migrations",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "mediarr_schema_migrations",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "import_artifacts",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "job_runs",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "quality_profiles_json",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "subtitle_providers",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "settings_definitions",
        classification: BackupTableClassification::Rebuild,
    },
    BackupTableCatalogEntry {
        table: "title_search_terms",
        classification: BackupTableClassification::Rebuild,
    },
    BackupTableCatalogEntry {
        table: "title_search_spellfix",
        classification: BackupTableClassification::Rebuild,
    },
    BackupTableCatalogEntry {
        table: "blocklist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "collection_external_ids",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "collections",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "domain_events",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "download_clients",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "download_import_artifacts",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "download_jobs",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "download_queue_commands",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "download_submission_episode_links",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "download_submissions",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "entitlements",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "episode_external_ids",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "episodes",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "event_outboxes",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "event_subscriber_offsets",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "external_import_monitor_snapshots",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "external_subtitle_probe_cache",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "file_episode_map",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "history_events",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "imports",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "indexer_api_quotas",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "indexers",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "integration_tokens",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "libraries",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "library_probe_signatures",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "library_roots",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "library_scan_unmatched_items",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "media_files",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "notification_channels",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "notification_subscriptions",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "pending_releases",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "plugin_catalog_sources",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "plugin_catalog_status",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "plugin_installations",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "post_processing_script_runs",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "post_processing_scripts",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "push_subscriptions",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profile_audio_codec_allowlist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profile_audio_codec_blocklist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profile_quality_tiers",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profile_source_allowlist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profile_source_blocklist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profile_video_codec_allowlist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profile_video_codec_blocklist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quality_profiles",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "quarantine_items",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "release_decisions",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "release_download_attempts",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "releases",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "rule_set_history",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "rule_sets",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "scheduler_jobs",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "settings_values",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "subtitle_blocklist",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "subtitle_downloads",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "subtitle_provider_configs",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "title_aliases",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "title_external_ids",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "title_image_variants",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "title_images",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "titles",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "upgrades",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "user_app_permission_masks",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "user_entitlements",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "user_library_permission_masks",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "users",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "wanted_items",
        classification: BackupTableClassification::Export,
    },
    BackupTableCatalogEntry {
        table: "workflow_operations",
        classification: BackupTableClassification::Export,
    },
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupBundleInspectSummary {
    pub format_version: String,
    pub created_at: String,
    pub source_scryer_version: String,
    pub source_engine: String,
    pub source_migration_key: Option<String>,
    pub encrypted: bool,
    pub row_counts: BTreeMap<String, u64>,
}

impl BackupBundleInspectSummary {
    pub fn total_rows(&self) -> u64 {
        self.row_counts.values().copied().sum()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupBundleManifest {
    pub format_version: String,
    pub created_at: String,
    pub source_scryer_version: String,
    pub source_engine: String,
    pub source_migration_key: Option<String>,
    pub encrypted: bool,
    pub row_counts: BTreeMap<String, u64>,
    pub part_checksums: BTreeMap<String, String>,
}

impl BackupBundleManifest {
    pub fn summary(&self) -> BackupBundleInspectSummary {
        BackupBundleInspectSummary {
            format_version: self.format_version.clone(),
            created_at: self.created_at.clone(),
            source_scryer_version: self.source_scryer_version.clone(),
            source_engine: self.source_engine.clone(),
            source_migration_key: self.source_migration_key.clone(),
            encrypted: self.encrypted,
            row_counts: self.row_counts.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupInstanceSecrets {
    encryption_master_key: String,
    jwt_signing_secret: String,
    smg_registration_secret: Option<String>,
    smg_ca_cert: Option<String>,
    smg_gateway_url: Option<String>,
}

impl BackupInstanceSecrets {
    pub fn from_export_secrets(secrets: BackupExportSecrets) -> Self {
        Self {
            encryption_master_key: secrets.encryption_master_key,
            jwt_signing_secret: secrets.jwt_signing_secret,
            smg_registration_secret: secrets.smg_registration_secret,
            smg_ca_cert: secrets.smg_ca_cert,
            smg_gateway_url: secrets.smg_gateway_url,
        }
    }

    pub fn to_env_file(&self) -> String {
        let mut output = String::new();
        push_env_assignment(
            &mut output,
            "SCRYER_ENCRYPTION_KEY",
            &self.encryption_master_key,
        );
        push_env_assignment(
            &mut output,
            "SCRYER_JWT_SIGNING_SECRET",
            &self.jwt_signing_secret,
        );
        if let Some(value) = self.smg_registration_secret.as_deref() {
            push_env_assignment(&mut output, "SCRYER_SMG_REGISTRATION_SECRET", value);
        }
        if let Some(value) = self.smg_ca_cert.as_deref() {
            push_env_assignment(&mut output, "SCRYER_SMG_CA_CERT", value);
        }
        if let Some(value) = self.smg_gateway_url.as_deref() {
            push_env_assignment(&mut output, "SCRYER_METADATA_GATEWAY_GRAPHQL_URL", value);
        }
        output
    }
}

#[derive(Clone, Debug)]
pub struct BackupExportSecrets {
    pub encryption_master_key: String,
    pub jwt_signing_secret: String,
    pub smg_registration_secret: Option<String>,
    pub smg_ca_cert: Option<String>,
    pub smg_gateway_url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct BackupBundleExportRequest {
    pub output_path: PathBuf,
    pub passphrase: Option<String>,
    pub source_migration_key: Option<String>,
    pub source_scryer_version: String,
    pub source_engine: String,
    pub secrets: BackupExportSecrets,
}

#[derive(Clone, Debug)]
pub struct BackupExportOutcome {
    pub summary: BackupBundleInspectSummary,
}

pub struct BackupBundleStaging {
    staging: TempDir,
    row_counts: BTreeMap<String, u64>,
    part_checksums: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct BackupRestorePreparedBundle {
    summary: BackupBundleInspectSummary,
    instance_secrets_env: String,
}

impl BackupRestorePreparedBundle {
    pub fn from_summary_and_instance_secrets_env(
        summary: BackupBundleInspectSummary,
        instance_secrets_env: String,
    ) -> Self {
        Self {
            summary,
            instance_secrets_env,
        }
    }

    pub fn summary(&self) -> &BackupBundleInspectSummary {
        &self.summary
    }

    pub fn instance_secrets_env(&self) -> String {
        self.instance_secrets_env.clone()
    }
}

impl BackupBundleStaging {
    pub fn new() -> AppResult<Self> {
        let staging = tempfile::tempdir().map_err(|error| {
            AppError::Repository(format!("failed to create backup staging dir: {error}"))
        })?;
        let tables_dir = staging.path().join(TABLES_DIRNAME);
        std::fs::create_dir_all(&tables_dir).map_err(|error| {
            AppError::Repository(format!("failed to create tables staging dir: {error}"))
        })?;

        Ok(Self {
            staging,
            row_counts: BTreeMap::new(),
            part_checksums: BTreeMap::new(),
        })
    }

    pub fn tables_dir(&self) -> PathBuf {
        self.staging.path().join(TABLES_DIRNAME)
    }

    pub fn record_table_part(&mut self, table: &str, row_count: u64) -> AppResult<()> {
        self.row_counts.insert(table.to_string(), row_count);
        let rel_path = format!("{TABLES_DIRNAME}/{table}.ndjson.zst");
        let checksum = checksum_hex(self.staging.path().join(&rel_path))?;
        self.part_checksums.insert(rel_path, checksum);
        Ok(())
    }

    pub fn finish(mut self, request: BackupBundleExportRequest) -> AppResult<BackupExportOutcome> {
        let instance_secrets = BackupInstanceSecrets::from_export_secrets(request.secrets);
        let instance_secrets_path = self.staging.path().join(INSTANCE_SECRETS_FILENAME);
        write_json_file(&instance_secrets_path, &instance_secrets)?;
        self.part_checksums.insert(
            INSTANCE_SECRETS_FILENAME.to_string(),
            checksum_hex(&instance_secrets_path)?,
        );

        let manifest = BackupBundleManifest {
            format_version: BACKUP_FORMAT_VERSION.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            source_scryer_version: request.source_scryer_version,
            source_engine: request.source_engine,
            source_migration_key: request.source_migration_key,
            encrypted: request.passphrase.is_some(),
            row_counts: self.row_counts,
            part_checksums: self.part_checksums,
        };
        let manifest_path = self.staging.path().join(MANIFEST_FILENAME);
        write_json_file(&manifest_path, &manifest)?;

        let temp_payload_path = self.staging.path().join("bundle.tar.zst");
        write_bundle_payload(self.staging.path(), &temp_payload_path)?;

        if let Some(passphrase) = request.passphrase.as_deref() {
            encrypt_payload_with_aead(&temp_payload_path, &request.output_path, passphrase)?;
        } else {
            move_with_permissions(&temp_payload_path, &request.output_path)?;
        }

        ensure_owner_only_permissions(&request.output_path)?;

        Ok(BackupExportOutcome {
            summary: manifest.summary(),
        })
    }
}

pub fn inspect_backup_bundle(
    bundle_path: &Path,
    passphrase: Option<&str>,
) -> AppResult<BackupBundleInspectSummary> {
    let extracted = extract_bundle_to_tempdir(bundle_path, passphrase)?;
    let manifest = load_manifest(extracted.path())?;
    validate_extracted_bundle(extracted.path(), &manifest)?;
    Ok(manifest.summary())
}

pub struct BackupBundleRestorePayload {
    extracted: TempDir,
    manifest: BackupBundleManifest,
}

impl BackupBundleRestorePayload {
    pub fn tables_dir(&self) -> PathBuf {
        self.extracted.path().join(TABLES_DIRNAME)
    }

    pub fn manifest(&self) -> &BackupBundleManifest {
        &self.manifest
    }

    pub fn summary(&self) -> BackupBundleInspectSummary {
        self.manifest.summary()
    }

    pub fn instance_secrets_env(&self) -> AppResult<String> {
        Ok(load_instance_secrets(self.extracted.path())?.to_env_file())
    }
}

pub fn prepare_backup_restore_payload(
    bundle_path: &Path,
    passphrase: Option<&str>,
) -> AppResult<BackupBundleRestorePayload> {
    let extracted = extract_bundle_to_tempdir(bundle_path, passphrase)?;
    let manifest = load_manifest(extracted.path())?;
    validate_extracted_bundle(extracted.path(), &manifest)?;

    Ok(BackupBundleRestorePayload {
        extracted,
        manifest,
    })
}

fn push_env_assignment(output: &mut String, key: &str, value: &str) {
    output.push_str(key);
    output.push('=');
    output.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            _ => output.push(ch),
        }
    }
    output.push_str("\"\n");
}

fn write_json_file(path: &Path, value: &impl Serialize) -> AppResult<()> {
    let file = File::create(path).map_err(|error| {
        AppError::Repository(format!("failed to create {}: {error}", path.display()))
    })?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, value).map_err(|error| {
        AppError::Repository(format!("failed to serialize {}: {error}", path.display()))
    })
}

fn checksum_hex(path: impl AsRef<Path>) -> AppResult<String> {
    let mut file = File::open(path.as_ref()).map_err(|error| {
        AppError::Repository(format!(
            "failed to open {} for checksum: {error}",
            path.as_ref().display()
        ))
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            AppError::Repository(format!(
                "failed to read {} for checksum: {error}",
                path.as_ref().display()
            ))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn move_with_permissions(source: &Path, dest: &Path) -> AppResult<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppError::Repository(format!("failed to create {}: {error}", parent.display()))
        })?;
    }
    std::fs::rename(source, dest)
        .or_else(|_| {
            std::fs::copy(source, dest)?;
            std::fs::remove_file(source)
        })
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to move staged backup {} to {}: {error}",
                source.display(),
                dest.display()
            ))
        })?;
    Ok(())
}

fn write_bundle_payload(stage_dir: &Path, output_path: &Path) -> AppResult<()> {
    let file = File::create(output_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to create payload {}: {error}",
            output_path.display()
        ))
    })?;
    let encoder = zstd::Encoder::new(file, 3)
        .map_err(|error| AppError::Repository(format!("failed to start zstd encoder: {error}")))?;
    let writer = encoder.auto_finish();
    let mut tar = tar::Builder::new(writer);
    tar.append_path_with_name(stage_dir.join(MANIFEST_FILENAME), MANIFEST_FILENAME)
        .map_err(|error| {
            AppError::Repository(format!("failed to append manifest to tar: {error}"))
        })?;
    tar.append_path_with_name(
        stage_dir.join(INSTANCE_SECRETS_FILENAME),
        INSTANCE_SECRETS_FILENAME,
    )
    .map_err(|error| AppError::Repository(format!("failed to append secrets to tar: {error}")))?;
    tar.append_dir_all(TABLES_DIRNAME, stage_dir.join(TABLES_DIRNAME))
        .map_err(|error| {
            AppError::Repository(format!("failed to append tables to tar: {error}"))
        })?;
    tar.finish().map_err(|error| {
        AppError::Repository(format!("failed to finalize tar payload: {error}"))
    })?;
    Ok(())
}

fn encrypt_payload_with_aead(
    input_path: &Path,
    output_path: &Path,
    passphrase: &str,
) -> AppResult<()> {
    let input = File::open(input_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to open staged payload {}: {error}",
            input_path.display()
        ))
    })?;
    let output = File::create(output_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to create encrypted bundle {}: {error}",
            output_path.display()
        ))
    })?;
    let header = EncryptedBundleHeaderV1 {
        metadata: BackupEncryptionMetadataV1::generate()?,
    };
    let version = BACKUP_ENCRYPTION_VERSION_1;
    let metadata_bytes = header.metadata.to_bytes();
    let key_bytes = derive_backup_encryption_key(passphrase, &header.metadata.salt)?;
    let key = make_backup_aead_key(&key_bytes)?;

    let mut reader = BufReader::new(input);
    let mut writer = BufWriter::new(output);
    writer.write_all(&ENCRYPTED_BUNDLE_MAGIC).map_err(|error| {
        AppError::Repository(format!("failed to write encrypted backup header: {error}"))
    })?;
    writer.write_all(&[version]).map_err(|error| {
        AppError::Repository(format!("failed to write encrypted backup version: {error}"))
    })?;
    writer
        .write_all(&(metadata_bytes.len() as u32).to_be_bytes())
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to write encrypted backup metadata length: {error}"
            ))
        })?;
    writer.write_all(&metadata_bytes).map_err(|error| {
        AppError::Repository(format!(
            "failed to write encrypted backup metadata: {error}"
        ))
    })?;

    let mut buffer = vec![0_u8; BACKUP_ENCRYPTION_CHUNK_SIZE];
    let mut chunk_index = 0_u64;
    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            AppError::Repository(format!(
                "failed to read staged payload for encryption: {error}"
            ))
        })?;
        if read == 0 {
            break;
        }

        let mut in_out = buffer[..read].to_vec();
        let nonce = chunk_nonce(header.metadata.nonce_prefix, chunk_index);
        let aad = chunk_aad(version, &metadata_bytes, chunk_index);
        key.seal_in_place_append_tag(nonce, Aad::from(aad.as_slice()), &mut in_out)
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to encrypt backup chunk {chunk_index}: {error}"
                ))
            })?;

        let chunk_len = u32::try_from(in_out.len()).map_err(|_| {
            AppError::Repository("encrypted backup chunk length exceeds u32".into())
        })?;
        writer
            .write_all(&chunk_len.to_be_bytes())
            .and_then(|_| writer.write_all(&in_out))
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to write encrypted backup chunk {chunk_index}: {error}"
                ))
            })?;
        chunk_index = chunk_index
            .checked_add(1)
            .ok_or_else(|| AppError::Repository("backup chunk index overflowed".into()))?;
    }

    writer.flush().map_err(|error| {
        AppError::Repository(format!("failed to finalize encrypted bundle: {error}"))
    })?;
    Ok(())
}

fn extract_bundle_to_tempdir(bundle_path: &Path, passphrase: Option<&str>) -> AppResult<TempDir> {
    let tempdir = tempfile::tempdir().map_err(|error| {
        AppError::Repository(format!(
            "failed to create restore staging directory: {error}"
        ))
    })?;

    let payload_path = tempdir.path().join("payload.tar.zst");
    if parse_encrypted_bundle_header(bundle_path)?.is_some() {
        let passphrase = passphrase.ok_or_else(|| {
            AppError::Validation("this backup bundle is encrypted and requires a password".into())
        })?;
        decrypt_payload_with_aead(bundle_path, &payload_path, passphrase)?;
    } else {
        std::fs::copy(bundle_path, &payload_path).map_err(|error| {
            AppError::Repository(format!(
                "failed to stage bundle payload from {}: {error}",
                bundle_path.display()
            ))
        })?;
    }

    let payload_file = File::open(&payload_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to open staged payload {}: {error}",
            payload_path.display()
        ))
    })?;
    let decoder = zstd::Decoder::new(BufReader::new(payload_file)).map_err(|error| {
        AppError::Validation(format!("backup payload is not valid zstd: {error}"))
    })?;
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(tempdir.path()).map_err(|error| {
        AppError::Validation(format!(
            "backup payload is not a valid tar archive: {error}"
        ))
    })?;

    Ok(tempdir)
}

fn decrypt_payload_with_aead(
    input_path: &Path,
    output_path: &Path,
    passphrase: &str,
) -> AppResult<()> {
    let input = File::open(input_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to open encrypted bundle {}: {error}",
            input_path.display()
        ))
    })?;
    let mut reader = BufReader::new(input);
    let header = parse_encrypted_bundle_header_from_reader(&mut reader)?.ok_or_else(|| {
        AppError::Validation("backup bundle is not a supported encrypted backup".into())
    })?;
    let metadata_bytes = header.metadata.to_bytes();
    let key_bytes = derive_backup_encryption_key(passphrase, &header.metadata.salt)?;
    let key = make_backup_aead_key(&key_bytes)?;

    let output = File::create(output_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to create decrypted payload {}: {error}",
            output_path.display()
        ))
    })?;
    let mut writer = BufWriter::new(output);
    let mut chunk_index = 0_u64;

    while let Some(ciphertext_len) = read_encrypted_chunk_len(&mut reader)? {
        let mut in_out = vec![0_u8; ciphertext_len];
        reader.read_exact(&mut in_out).map_err(|error| {
            AppError::Validation(format!(
                "encrypted backup payload is truncated or invalid: {error}"
            ))
        })?;

        let nonce = chunk_nonce(header.metadata.nonce_prefix, chunk_index);
        let aad = chunk_aad(BACKUP_ENCRYPTION_VERSION_1, &metadata_bytes, chunk_index);
        let plaintext = key
            .open_in_place(nonce, Aad::from(aad.as_slice()), &mut in_out)
            .map_err(|_| {
                AppError::Validation(
                    "failed to decrypt backup bundle: wrong password or corrupted data".into(),
                )
            })?;

        writer.write_all(plaintext).map_err(|error| {
            AppError::Repository(format!(
                "failed to write decrypted backup payload chunk {chunk_index}: {error}"
            ))
        })?;
        chunk_index = chunk_index
            .checked_add(1)
            .ok_or_else(|| AppError::Repository("backup chunk index overflowed".into()))?;
    }

    writer.flush().map_err(|error| {
        AppError::Repository(format!("failed to write decrypted backup payload: {error}"))
    })?;
    Ok(())
}

fn parse_encrypted_bundle_header(bundle_path: &Path) -> AppResult<Option<EncryptedBundleHeaderV1>> {
    let input = File::open(bundle_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to open bundle {}: {error}",
            bundle_path.display()
        ))
    })?;
    let mut reader = BufReader::new(input);
    parse_encrypted_bundle_header_from_reader(&mut reader)
}

fn parse_encrypted_bundle_header_from_reader(
    reader: &mut impl Read,
) -> AppResult<Option<EncryptedBundleHeaderV1>> {
    let mut magic = [0_u8; ENCRYPTED_BUNDLE_MAGIC.len()];
    let read = reader.read(&mut magic).map_err(|error| {
        AppError::Repository(format!("failed to read encrypted backup header: {error}"))
    })?;
    if read != ENCRYPTED_BUNDLE_MAGIC.len() || magic != ENCRYPTED_BUNDLE_MAGIC {
        return Ok(None);
    }

    let mut version = [0_u8; 1];
    reader.read_exact(&mut version).map_err(|error| {
        AppError::Validation(format!("encrypted backup header is truncated: {error}"))
    })?;
    if version[0] != BACKUP_ENCRYPTION_VERSION_1 {
        return Err(AppError::Validation(format!(
            "unsupported encrypted backup version {}",
            version[0]
        )));
    }

    let mut metadata_len = [0_u8; 4];
    reader.read_exact(&mut metadata_len).map_err(|error| {
        AppError::Validation(format!(
            "encrypted backup metadata header is truncated: {error}"
        ))
    })?;
    let metadata_len = u32::from_be_bytes(metadata_len) as usize;
    if metadata_len != BACKUP_ENCRYPTION_METADATA_V1_LEN {
        return Err(AppError::Validation(
            "backup encryption metadata is invalid".into(),
        ));
    }

    let mut metadata_bytes = [0_u8; BACKUP_ENCRYPTION_METADATA_V1_LEN];
    reader.read_exact(&mut metadata_bytes).map_err(|error| {
        AppError::Validation(format!("encrypted backup metadata is truncated: {error}"))
    })?;

    Ok(Some(EncryptedBundleHeaderV1 {
        metadata: BackupEncryptionMetadataV1::from_bytes(&metadata_bytes)?,
    }))
}

fn derive_backup_encryption_key(
    passphrase: &str,
    salt: &[u8; BACKUP_ENCRYPTION_SALT_LEN],
) -> AppResult<[u8; BACKUP_ENCRYPTION_KEY_LEN]> {
    let params = Params::new(
        BACKUP_ENCRYPTION_ARGON2_M_COST_KIB,
        BACKUP_ENCRYPTION_ARGON2_T_COST,
        BACKUP_ENCRYPTION_ARGON2_P_COST,
        Some(BACKUP_ENCRYPTION_KEY_LEN),
    )
    .map_err(|error| {
        AppError::Repository(format!(
            "failed to configure backup encryption KDF parameters: {error}"
        ))
    })?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0_u8; BACKUP_ENCRYPTION_KEY_LEN];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|error| {
            AppError::Validation(format!(
                "failed to derive backup encryption key from password: {error}"
            ))
        })?;
    Ok(key)
}

fn make_backup_aead_key(key_bytes: &[u8; BACKUP_ENCRYPTION_KEY_LEN]) -> AppResult<LessSafeKey> {
    let unbound = UnboundKey::new(&AES_256_GCM, key_bytes).map_err(|error| {
        AppError::Repository(format!(
            "failed to construct backup encryption key: {error}"
        ))
    })?;
    Ok(LessSafeKey::new(unbound))
}

fn chunk_nonce(nonce_prefix: [u8; BACKUP_ENCRYPTION_NONCE_PREFIX_LEN], chunk_index: u64) -> Nonce {
    let mut nonce = [0_u8; 12];
    nonce[..BACKUP_ENCRYPTION_NONCE_PREFIX_LEN].copy_from_slice(&nonce_prefix);
    nonce[BACKUP_ENCRYPTION_NONCE_PREFIX_LEN..].copy_from_slice(&chunk_index.to_be_bytes());
    Nonce::assume_unique_for_key(nonce)
}

fn chunk_aad(version: u8, metadata_bytes: &[u8], chunk_index: u64) -> Vec<u8> {
    let mut aad = Vec::with_capacity(ENCRYPTED_BUNDLE_MAGIC.len() + 1 + metadata_bytes.len() + 8);
    aad.extend_from_slice(&ENCRYPTED_BUNDLE_MAGIC);
    aad.push(version);
    aad.extend_from_slice(metadata_bytes);
    aad.extend_from_slice(&chunk_index.to_be_bytes());
    aad
}

fn read_encrypted_chunk_len(reader: &mut impl Read) -> AppResult<Option<usize>> {
    let mut len = [0_u8; 4];
    let read = reader.read(&mut len[..1]).map_err(|error| {
        AppError::Validation(format!(
            "encrypted backup payload length is invalid: {error}"
        ))
    })?;
    if read == 0 {
        return Ok(None);
    }

    reader.read_exact(&mut len[1..]).map_err(|error| {
        AppError::Validation(format!(
            "encrypted backup payload length is truncated: {error}"
        ))
    })?;
    let chunk_len = u32::from_be_bytes(len) as usize;
    if !(BACKUP_ENCRYPTION_TAG_LEN..=BACKUP_ENCRYPTION_MAX_CIPHERTEXT_CHUNK_LEN)
        .contains(&chunk_len)
    {
        return Err(AppError::Validation(
            "encrypted backup payload length is invalid".into(),
        ));
    }
    Ok(Some(chunk_len))
}

fn load_manifest(root: &Path) -> AppResult<BackupBundleManifest> {
    let path = root.join(MANIFEST_FILENAME);
    let file = File::open(&path)
        .map_err(|error| AppError::Validation(format!("backup manifest missing: {error}")))?;
    serde_json::from_reader(BufReader::new(file))
        .map_err(|error| AppError::Validation(format!("backup manifest is invalid: {error}")))
}

fn load_instance_secrets(root: &Path) -> AppResult<BackupInstanceSecrets> {
    let path = root.join(INSTANCE_SECRETS_FILENAME);
    let file = File::open(&path).map_err(|error| {
        AppError::Validation(format!("backup secrets payload missing: {error}"))
    })?;
    serde_json::from_reader(BufReader::new(file)).map_err(|error| {
        AppError::Validation(format!("backup secrets payload is invalid: {error}"))
    })
}

fn validate_extracted_bundle(root: &Path, manifest: &BackupBundleManifest) -> AppResult<()> {
    if manifest.format_version != BACKUP_FORMAT_VERSION {
        return Err(AppError::Validation(format!(
            "unsupported backup format version {}",
            manifest.format_version
        )));
    }

    for (part, expected_checksum) in &manifest.part_checksums {
        let actual_checksum = checksum_hex(root.join(part))?;
        if &actual_checksum != expected_checksum {
            return Err(AppError::Validation(format!(
                "backup checksum mismatch for {part}"
            )));
        }
    }

    Ok(())
}

fn ensure_owner_only_permissions(path: &Path) -> AppResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let permissions = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, permissions).map_err(|error| {
            AppError::Repository(format!(
                "failed to set permissions on {}: {error}",
                path.display()
            ))
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};

    #[test]
    fn env_file_writer_escapes_multiline_values() {
        let secrets = BackupInstanceSecrets {
            encryption_master_key: "enc".into(),
            jwt_signing_secret: "jwt".into(),
            smg_registration_secret: Some("reg".into()),
            smg_ca_cert: Some("line1\nline2".into()),
            smg_gateway_url: Some("https://smg.example/graphql".into()),
        };

        let env_file = secrets.to_env_file();
        assert!(env_file.contains("SCRYER_ENCRYPTION_KEY=\"enc\""));
        assert!(env_file.contains("SCRYER_SMG_CA_CERT=\"line1\\nline2\""));
    }

    #[test]
    fn encrypted_backup_round_trip_uses_versioned_header() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("roundtrip.scryer-backup.enc");
        let passphrase = "backup-passphrase";
        write_test_bundle(&bundle_path, Some(passphrase)).expect("write bundle");

        let summary = inspect_backup_bundle(&bundle_path, Some(passphrase)).expect("inspect");
        assert!(summary.encrypted);

        let bytes = std::fs::read(&bundle_path).expect("read bundle");
        assert_eq!(
            &bytes[..ENCRYPTED_BUNDLE_MAGIC.len()],
            &ENCRYPTED_BUNDLE_MAGIC
        );
        assert_eq!(
            bytes[ENCRYPTED_BUNDLE_MAGIC.len()],
            BACKUP_ENCRYPTION_VERSION_1
        );
    }

    #[test]
    fn encrypted_backup_rejects_wrong_passphrase() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("wrong-pass.scryer-backup.enc");
        write_test_bundle(&bundle_path, Some("correct")).expect("write bundle");

        let error = inspect_backup_bundle(&bundle_path, Some("wrong"))
            .expect_err("wrong password should fail");
        assert!(
            error
                .to_string()
                .contains("failed to decrypt backup bundle")
        );
    }

    #[test]
    fn encrypted_backup_rejects_unknown_version() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("unknown-version.scryer-backup.enc");
        write_test_bundle(&bundle_path, Some("correct")).expect("write bundle");

        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&bundle_path)
            .expect("open bundle");
        file.seek(SeekFrom::Start(ENCRYPTED_BUNDLE_MAGIC.len() as u64))
            .expect("seek");
        file.write_all(&[BACKUP_ENCRYPTION_VERSION_1 + 1])
            .expect("write version");

        let error = inspect_backup_bundle(&bundle_path, Some("correct"))
            .expect_err("unknown version should fail");
        assert!(
            error
                .to_string()
                .contains("unsupported encrypted backup version")
        );
    }

    #[test]
    fn encrypted_backup_rejects_truncated_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("truncated-metadata.scryer-backup.enc");
        write_test_bundle(&bundle_path, Some("correct")).expect("write bundle");

        let mut bytes = std::fs::read(&bundle_path).expect("read bundle");
        bytes.truncate(ENCRYPTED_BUNDLE_MAGIC.len() + 1 + 4 + 3);
        std::fs::write(&bundle_path, bytes).expect("rewrite bundle");

        let error = inspect_backup_bundle(&bundle_path, Some("correct"))
            .expect_err("truncated metadata should fail");
        assert!(error.to_string().contains("metadata is truncated"));
    }

    #[test]
    fn encrypted_backup_rejects_invalid_metadata_length() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp
            .path()
            .join("invalid-metadata-length.scryer-backup.enc");
        write_test_bundle(&bundle_path, Some("correct")).expect("write bundle");

        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&bundle_path)
            .expect("open bundle");
        file.seek(SeekFrom::Start((ENCRYPTED_BUNDLE_MAGIC.len() + 1) as u64))
            .expect("seek");
        file.write_all(&(0_u32).to_be_bytes())
            .expect("write metadata length");

        let error = inspect_backup_bundle(&bundle_path, Some("correct"))
            .expect_err("invalid metadata length should fail");
        assert!(error.to_string().contains("metadata is invalid"));
    }

    #[test]
    fn encrypted_backup_rejects_truncated_chunk() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("truncated-chunk.scryer-backup.enc");
        write_test_bundle(&bundle_path, Some("correct")).expect("write bundle");

        let mut bytes = std::fs::read(&bundle_path).expect("read bundle");
        bytes.pop();
        std::fs::write(&bundle_path, bytes).expect("rewrite bundle");

        let error = inspect_backup_bundle(&bundle_path, Some("correct"))
            .expect_err("truncated chunk should fail");
        assert!(
            error
                .to_string()
                .contains("payload is truncated or invalid")
        );
    }

    #[test]
    fn encrypted_backup_round_trip_handles_exact_chunk_boundary() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("exact-boundary.scryer-backup.enc");
        let passphrase = "exact-boundary-pass";
        write_test_bundle_with_payload_size(
            &bundle_path,
            Some(passphrase),
            BACKUP_ENCRYPTION_CHUNK_SIZE,
        )
        .expect("write bundle");

        let summary = inspect_backup_bundle(&bundle_path, Some(passphrase)).expect("inspect");
        assert_eq!(summary.row_counts.get("titles"), Some(&1));
    }

    #[test]
    fn encrypted_backup_round_trip_spans_multiple_chunks() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("multi-chunk.scryer-backup.enc");
        let passphrase = "multi-chunk-pass";
        write_test_bundle_with_payload_size(
            &bundle_path,
            Some(passphrase),
            BACKUP_ENCRYPTION_CHUNK_SIZE + 1,
        )
        .expect("write bundle");

        let summary = inspect_backup_bundle(&bundle_path, Some(passphrase)).expect("inspect");
        assert_eq!(summary.row_counts.get("titles"), Some(&1));
    }

    #[test]
    fn encrypted_backup_rejects_oversized_chunk_length() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_path = temp.path().join("oversized-chunk.scryer-backup.enc");
        write_test_bundle(&bundle_path, Some("correct")).expect("write bundle");

        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&bundle_path)
            .expect("open bundle");
        let chunk_len_offset =
            (ENCRYPTED_BUNDLE_MAGIC.len() + 1 + 4 + BACKUP_ENCRYPTION_METADATA_V1_LEN) as u64;
        file.seek(SeekFrom::Start(chunk_len_offset)).expect("seek");
        file.write_all(
            &(u32::try_from(BACKUP_ENCRYPTION_MAX_CIPHERTEXT_CHUNK_LEN).unwrap() + 1).to_be_bytes(),
        )
        .expect("write chunk length");

        let error = inspect_backup_bundle(&bundle_path, Some("correct"))
            .expect_err("oversized chunk should fail");
        assert!(error.to_string().contains("payload length is invalid"));
    }

    fn write_test_bundle(output_path: &Path, passphrase: Option<&str>) -> AppResult<()> {
        write_test_bundle_with_payload_size(output_path, passphrase, 64)
    }

    fn write_test_bundle_with_payload_size(
        output_path: &Path,
        passphrase: Option<&str>,
        payload_size: usize,
    ) -> AppResult<()> {
        let mut staging = BackupBundleStaging::new()?;
        let table_path = staging.tables_dir().join("titles.ndjson.zst");
        write_zstd_payload(&table_path, payload_size)?;
        staging.record_table_part("titles", 1)?;
        staging.finish(BackupBundleExportRequest {
            output_path: output_path.to_path_buf(),
            passphrase: passphrase.map(str::to_string),
            source_migration_key: Some("0112".to_string()),
            source_scryer_version: "test".to_string(),
            source_engine: "sqlite".to_string(),
            secrets: BackupExportSecrets {
                encryption_master_key: "master-key".to_string(),
                jwt_signing_secret: "jwt-secret".to_string(),
                smg_registration_secret: Some("smg-secret".to_string()),
                smg_ca_cert: None,
                smg_gateway_url: None,
            },
        })?;
        Ok(())
    }

    fn write_zstd_payload(path: &Path, payload_size: usize) -> AppResult<()> {
        let file = File::create(path).map_err(|error| {
            AppError::Repository(format!("failed to create test payload: {error}"))
        })?;
        let mut encoder = zstd::Encoder::new(file, 1).map_err(|error| {
            AppError::Repository(format!("failed to create test zstd encoder: {error}"))
        })?;
        let payload = vec![b'x'; payload_size];
        encoder.write_all(&payload).map_err(|error| {
            AppError::Repository(format!("failed to write test payload: {error}"))
        })?;
        encoder.finish().map_err(|error| {
            AppError::Repository(format!("failed to finish test payload: {error}"))
        })?;
        Ok(())
    }
}
