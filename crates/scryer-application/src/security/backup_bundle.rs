use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};

use age::secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::{AppError, AppResult};

pub const BACKUP_FORMAT_VERSION: &str = "scryer-backup-bundle-v1";
pub const BACKUP_PLAINTEXT_EXTENSION: &str = ".scryer-backup.tar.zst";
pub const BACKUP_ENCRYPTED_EXTENSION: &str = ".scryer-backup.age";

const INSTANCE_SECRETS_FILENAME: &str = "instance-secrets.json";
const MANIFEST_FILENAME: &str = "manifest.json";
const TABLES_DIRNAME: &str = "tables";
pub const BLOB_MARKER_TYPE: &str = "__scryer_type";
pub const BLOB_MARKER_BASE64: &str = "base64";
pub const EXPORT_BATCH_SIZE: i64 = 1_000;

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
            encrypt_payload_with_age(&temp_payload_path, &request.output_path, passphrase)?;
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

fn encrypt_payload_with_age(
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
    let encryptor =
        age::Encryptor::with_user_passphrase(SecretString::from(passphrase.to_string()));
    let mut writer = encryptor
        .wrap_output(BufWriter::new(output))
        .map_err(|error| AppError::Repository(format!("failed to wrap age output: {error}")))?;
    std::io::copy(&mut BufReader::new(input), &mut writer).map_err(|error| {
        AppError::Repository(format!("failed to write encrypted bundle: {error}"))
    })?;
    writer.finish().map_err(|error| {
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
    if is_age_bundle(bundle_path)? {
        let passphrase = passphrase.ok_or_else(|| {
            AppError::Validation("this backup bundle is encrypted and requires a password".into())
        })?;
        decrypt_payload_with_age(bundle_path, &payload_path, passphrase)?;
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

fn decrypt_payload_with_age(
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
    let decryptor = age::Decryptor::new(BufReader::new(input)).map_err(|error| {
        AppError::Validation(format!("backup bundle is not a valid age payload: {error}"))
    })?;

    if !decryptor.is_scrypt() {
        return Err(AppError::Validation(
            "backup bundle uses an unsupported age recipient type".into(),
        ));
    }

    let identity = age::scrypt::Identity::new(SecretString::from(passphrase.to_string()));
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|error| {
            AppError::Validation(format!("failed to decrypt backup bundle: {error}"))
        })?;

    let output = File::create(output_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to create decrypted payload {}: {error}",
            output_path.display()
        ))
    })?;
    std::io::copy(&mut reader, &mut BufWriter::new(output)).map_err(|error| {
        AppError::Repository(format!("failed to write decrypted backup payload: {error}"))
    })?;
    Ok(())
}

fn is_age_bundle(bundle_path: &Path) -> AppResult<bool> {
    let mut file = File::open(bundle_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to open bundle {}: {error}",
            bundle_path.display()
        ))
    })?;
    let mut prefix = [0_u8; 32];
    let read = file.read(&mut prefix).map_err(|error| {
        AppError::Repository(format!(
            "failed to read bundle {}: {error}",
            bundle_path.display()
        ))
    })?;
    let prefix = String::from_utf8_lossy(&prefix[..read]);
    Ok(prefix.starts_with("age-encryption.org/"))
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
}
