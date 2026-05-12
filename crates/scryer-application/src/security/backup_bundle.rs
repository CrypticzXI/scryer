use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::Path;

use age::secrecy::SecretString;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use sqlx::{Column, Row, TypeInfo, ValueRef};
use tempfile::TempDir;

use crate::{AppError, AppResult};

pub const BACKUP_FORMAT_VERSION: &str = "scryer-backup-bundle-v1";
pub const BACKUP_SOURCE_ENGINE_SQLITE: &str = "sqlite";
pub const BACKUP_PLAINTEXT_EXTENSION: &str = ".scryer-backup.tar.zst";
pub const BACKUP_ENCRYPTED_EXTENSION: &str = ".scryer-backup.age";

const INSTANCE_SECRETS_FILENAME: &str = "instance-secrets.json";
const MANIFEST_FILENAME: &str = "manifest.json";
const TABLES_DIRNAME: &str = "tables";
const BLOB_MARKER_TYPE: &str = "__scryer_type";
const BLOB_MARKER_BASE64: &str = "base64";
const EXPORT_BATCH_SIZE: i64 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackupTableClassification {
    Export,
    Rebuild,
    Ignore,
}

#[derive(Clone, Copy, Debug)]
struct BackupTableCatalogEntry {
    table: &'static str,
    classification: BackupTableClassification,
}

const BACKUP_TABLE_CATALOG: &[BackupTableCatalogEntry] = &[
    BackupTableCatalogEntry {
        table: "_sqlx_migrations",
        classification: BackupTableClassification::Ignore,
    },
    BackupTableCatalogEntry {
        table: "mediarr_schema_migrations",
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
struct BackupBundleManifest {
    format_version: String,
    created_at: String,
    source_scryer_version: String,
    source_engine: String,
    source_migration_key: Option<String>,
    encrypted: bool,
    row_counts: BTreeMap<String, u64>,
    part_checksums: BTreeMap<String, String>,
}

impl BackupBundleManifest {
    fn summary(&self) -> BackupBundleInspectSummary {
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
struct BackupInstanceSecrets {
    encryption_master_key: String,
    jwt_signing_secret: String,
    smg_registration_secret: Option<String>,
    smg_ca_cert: Option<String>,
    smg_gateway_url: Option<String>,
}

impl BackupInstanceSecrets {
    fn to_env_file(&self) -> String {
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
pub(crate) struct BackupExportSecrets {
    pub encryption_master_key: String,
    pub jwt_signing_secret: String,
    pub smg_registration_secret: Option<String>,
    pub smg_ca_cert: Option<String>,
    pub smg_gateway_url: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct BackupExportOutcome {
    pub summary: BackupBundleInspectSummary,
}

#[derive(Clone, Debug)]
pub struct BackupRestorePreparedBundle {
    summary: BackupBundleInspectSummary,
    instance_secrets: BackupInstanceSecrets,
}

impl BackupRestorePreparedBundle {
    pub fn summary(&self) -> &BackupBundleInspectSummary {
        &self.summary
    }

    pub fn instance_secrets_env(&self) -> String {
        self.instance_secrets.to_env_file()
    }
}

pub(crate) async fn export_backup_bundle(
    db_path: &str,
    output_path: &Path,
    passphrase: Option<&str>,
    source_migration_key: Option<String>,
    source_scryer_version: &str,
    secrets: BackupExportSecrets,
) -> AppResult<BackupExportOutcome> {
    let staging = tempfile::tempdir().map_err(|error| {
        AppError::Repository(format!("failed to create backup staging dir: {error}"))
    })?;
    let tables_dir = staging.path().join(TABLES_DIRNAME);
    std::fs::create_dir_all(&tables_dir).map_err(|error| {
        AppError::Repository(format!("failed to create tables staging dir: {error}"))
    })?;

    let mut connect_options = db_connect_options(db_path)?;
    connect_options = connect_options.read_only(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connect_options)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to open source database for backup: {error}"
            ))
        })?;

    validate_backup_catalog(&pool).await?;
    let export_tables = ordered_export_tables(&pool).await?;
    let created_at = chrono::Utc::now().to_rfc3339();
    let encrypted = passphrase.is_some();

    let mut row_counts = BTreeMap::new();
    let mut part_checksums = BTreeMap::new();

    let mut conn = pool.acquire().await.map_err(|error| {
        AppError::Repository(format!(
            "failed to acquire source database connection: {error}"
        ))
    })?;
    sqlx::query("BEGIN")
        .execute(&mut *conn)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to begin backup snapshot: {error}"))
        })?;

    for table in &export_tables {
        let row_count = export_table_part(&mut conn, table, &tables_dir).await?;
        row_counts.insert(table.clone(), row_count);
        let rel_path = format!("{TABLES_DIRNAME}/{table}.ndjson.zst");
        let checksum = checksum_hex(staging.path().join(&rel_path))?;
        part_checksums.insert(rel_path, checksum);
    }

    sqlx::query("ROLLBACK")
        .execute(&mut *conn)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to close backup snapshot: {error}"))
        })?;
    drop(conn);
    pool.close().await;

    let instance_secrets = BackupInstanceSecrets {
        encryption_master_key: secrets.encryption_master_key,
        jwt_signing_secret: secrets.jwt_signing_secret,
        smg_registration_secret: secrets.smg_registration_secret,
        smg_ca_cert: secrets.smg_ca_cert,
        smg_gateway_url: secrets.smg_gateway_url,
    };
    let instance_secrets_path = staging.path().join(INSTANCE_SECRETS_FILENAME);
    write_json_file(&instance_secrets_path, &instance_secrets)?;
    part_checksums.insert(
        INSTANCE_SECRETS_FILENAME.to_string(),
        checksum_hex(&instance_secrets_path)?,
    );

    let manifest = BackupBundleManifest {
        format_version: BACKUP_FORMAT_VERSION.to_string(),
        created_at,
        source_scryer_version: source_scryer_version.to_string(),
        source_engine: BACKUP_SOURCE_ENGINE_SQLITE.to_string(),
        source_migration_key,
        encrypted,
        row_counts,
        part_checksums,
    };
    let manifest_path = staging.path().join(MANIFEST_FILENAME);
    write_json_file(&manifest_path, &manifest)?;

    let temp_payload_path = staging.path().join("bundle.tar.zst");
    write_bundle_payload(staging.path(), &temp_payload_path)?;

    if encrypted {
        encrypt_payload_with_age(
            &temp_payload_path,
            output_path,
            passphrase.unwrap_or_default(),
        )?;
    } else {
        move_with_permissions(&temp_payload_path, output_path)?;
    }

    ensure_owner_only_permissions(output_path)?;

    Ok(BackupExportOutcome {
        summary: manifest.summary(),
    })
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

pub async fn restore_backup_bundle_into_pool(
    pool: &sqlx::SqlitePool,
    bundle_path: &Path,
    passphrase: Option<&str>,
) -> AppResult<BackupRestorePreparedBundle> {
    let extracted = extract_bundle_to_tempdir(bundle_path, passphrase)?;
    let manifest = load_manifest(extracted.path())?;
    validate_extracted_bundle(extracted.path(), &manifest)?;

    validate_backup_catalog(pool).await?;
    let export_tables = ordered_export_tables(pool).await?;
    let expected_tables = export_tables.iter().cloned().collect::<BTreeSet<_>>();
    let manifest_tables = manifest.row_counts.keys().cloned().collect::<BTreeSet<_>>();
    if manifest_tables != expected_tables {
        return Err(AppError::Validation(
            "backup bundle table set does not match the current restore catalog".into(),
        ));
    }

    let mut conn = pool.acquire().await.map_err(|error| {
        AppError::Repository(format!("failed to acquire restore connection: {error}"))
    })?;

    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *conn)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to disable foreign keys for restore: {error}"
            ))
        })?;

    for table in export_tables.iter().rev() {
        let sql = format!("DELETE FROM {}", quote_identifier(table));
        sqlx::query(&sql)
            .execute(&mut *conn)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to clear restore table {table}: {error}"))
            })?;
    }

    let tables_dir = extracted.path().join(TABLES_DIRNAME);
    for table in &export_tables {
        import_table_part(
            &mut conn,
            table,
            &tables_dir.join(format!("{table}.ndjson.zst")),
        )
        .await?;
    }

    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to re-enable foreign keys for restore: {error}"
            ))
        })?;

    let violations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
        .fetch_one(&mut *conn)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to validate restored foreign keys: {error}"))
        })?;
    if violations != 0 {
        return Err(AppError::Validation(
            "restored database failed foreign key validation".into(),
        ));
    }

    for (table, expected_rows) in &manifest.row_counts {
        let sql = format!("SELECT COUNT(*) FROM {}", quote_identifier(table));
        let actual_rows: i64 = sqlx::query_scalar(&sql)
            .fetch_one(&mut *conn)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to validate restored table {table}: {error}"
                ))
            })?;
        if actual_rows as u64 != *expected_rows {
            return Err(AppError::Validation(format!(
                "restored table {table} row count mismatch: expected {expected_rows}, got {actual_rows}"
            )));
        }
    }

    let instance_secrets = load_instance_secrets(extracted.path())?;

    Ok(BackupRestorePreparedBundle {
        summary: manifest.summary(),
        instance_secrets,
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

fn db_connect_options(db_path: &str) -> AppResult<SqliteConnectOptions> {
    db_path.parse::<SqliteConnectOptions>().map_err(|error| {
        AppError::Repository(format!("invalid sqlite database path {db_path}: {error}"))
    })
}

async fn validate_backup_catalog(pool: &sqlx::SqlitePool) -> AppResult<()> {
    let actual_tables = application_tables(pool).await?;
    let mut classified = BTreeSet::new();
    for entry in BACKUP_TABLE_CATALOG {
        classified.insert(entry.table.to_string());
    }

    let unclassified = actual_tables
        .into_iter()
        .filter(|table| !classified.contains(table))
        .collect::<Vec<_>>();
    if !unclassified.is_empty() {
        return Err(AppError::Repository(format!(
            "backup catalog is missing classifications for tables: {}",
            unclassified.join(", ")
        )));
    }

    Ok(())
}

async fn application_tables(pool: &sqlx::SqlitePool) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT name
           FROM sqlite_master
          WHERE type = 'table'
          ORDER BY name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| AppError::Repository(format!("failed to inspect sqlite schema: {error}")))?;

    let mut tables = Vec::new();
    for row in rows {
        let table: String = row.try_get("name").map_err(|error| {
            AppError::Repository(format!("failed to decode sqlite schema row: {error}"))
        })?;
        if is_engine_internal_table(&table) {
            continue;
        }
        tables.push(table);
    }
    Ok(tables)
}

fn is_engine_internal_table(table: &str) -> bool {
    table.starts_with("sqlite_") || table.starts_with("title_search_spellfix_")
}

async fn ordered_export_tables(pool: &sqlx::SqlitePool) -> AppResult<Vec<String>> {
    let export_tables = BACKUP_TABLE_CATALOG
        .iter()
        .filter(|entry| entry.classification == BackupTableClassification::Export)
        .map(|entry| entry.table.to_string())
        .collect::<BTreeSet<_>>();

    let mut incoming = BTreeMap::<String, usize>::new();
    let mut outgoing = BTreeMap::<String, BTreeSet<String>>::new();
    for table in &export_tables {
        incoming.insert(table.clone(), 0);
        outgoing.insert(table.clone(), BTreeSet::new());
    }

    for table in &export_tables {
        let pragma = format!("PRAGMA foreign_key_list({})", quote_identifier(table));
        let rows = sqlx::query(&pragma)
            .fetch_all(pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to inspect foreign keys for {table}: {error}"
                ))
            })?;
        for row in rows {
            let referenced: String = row.try_get("table").map_err(|error| {
                AppError::Repository(format!(
                    "failed to inspect foreign key for {table}: {error}"
                ))
            })?;
            if !export_tables.contains(&referenced) {
                continue;
            }
            if outgoing
                .get_mut(&referenced)
                .expect("known table")
                .insert(table.clone())
            {
                *incoming.get_mut(table).expect("known table") += 1;
            }
        }
    }

    let mut ready = incoming
        .iter()
        .filter_map(|(table, count)| (*count == 0).then_some(table.clone()))
        .collect::<VecDeque<_>>();
    let mut ordered = Vec::new();

    while let Some(table) = ready.pop_front() {
        ordered.push(table.clone());
        let dependents = outgoing.get(&table).cloned().unwrap_or_default();
        for dependent in dependents {
            let count = incoming.get_mut(&dependent).expect("known dependent");
            *count -= 1;
            if *count == 0 {
                let insert_at = ready
                    .iter()
                    .position(|candidate| candidate > &dependent)
                    .unwrap_or(ready.len());
                ready.insert(insert_at, dependent.clone());
            }
        }
    }

    if ordered.len() != export_tables.len() {
        return Err(AppError::Repository(
            "backup catalog dependencies contain a cycle".into(),
        ));
    }

    Ok(ordered)
}

async fn export_table_part(
    conn: &mut sqlx::SqliteConnection,
    table: &str,
    tables_dir: &Path,
) -> AppResult<u64> {
    let order_by = table_row_order_clause(conn, table).await?;
    let sql = if order_by.is_empty() {
        format!("SELECT * FROM {}", quote_identifier(table))
    } else {
        format!(
            "SELECT * FROM {} ORDER BY {}",
            quote_identifier(table),
            order_by
        )
    };

    let output_path = tables_dir.join(format!("{table}.ndjson.zst"));
    let file = File::create(&output_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to create table export {}: {error}",
            output_path.display()
        ))
    })?;
    let encoder = zstd::Encoder::new(file, 3).map_err(|error| {
        AppError::Repository(format!("failed to start zstd encoder for {table}: {error}"))
    })?;
    let mut writer = BufWriter::new(encoder.auto_finish());

    let mut count = 0_u64;
    let mut offset = 0_i64;
    let paged_sql = format!("{sql} LIMIT ? OFFSET ?");
    loop {
        let rows = sqlx::query(&paged_sql)
            .bind(EXPORT_BATCH_SIZE)
            .bind(offset)
            .fetch_all(&mut *conn)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to export table {table}: {error}"))
            })?;

        if rows.is_empty() {
            break;
        }

        let row_count = rows.len() as i64;
        for row in rows {
            let value = encode_row(&row)?;
            serde_json::to_writer(&mut writer, &value).map_err(|error| {
                AppError::Repository(format!("failed to encode backup row for {table}: {error}"))
            })?;
            writer.write_all(b"\n").map_err(|error| {
                AppError::Repository(format!("failed to write backup row for {table}: {error}"))
            })?;
            count += 1;
        }
        offset += row_count;
    }

    writer.flush().map_err(|error| {
        AppError::Repository(format!("failed to flush table export for {table}: {error}"))
    })?;
    Ok(count)
}

async fn table_row_order_clause(
    executor: &mut sqlx::SqliteConnection,
    table: &str,
) -> AppResult<String> {
    let pragma = format!("PRAGMA table_info({})", quote_identifier(table));
    let rows = sqlx::query(&pragma)
        .fetch_all(&mut *executor)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to inspect table info for {table}: {error}"))
        })?;

    let mut pk_columns = rows
        .iter()
        .filter_map(|row| {
            let pk: i64 = row.try_get("pk").ok()?;
            let name: String = row.try_get("name").ok()?;
            (pk > 0).then_some((pk, name))
        })
        .collect::<Vec<_>>();
    pk_columns.sort_by_key(|(pk, _)| *pk);

    if !pk_columns.is_empty() {
        return Ok(pk_columns
            .into_iter()
            .map(|(_, column)| quote_identifier(&column))
            .collect::<Vec<_>>()
            .join(", "));
    }

    if rows
        .iter()
        .any(|row| row.try_get::<String, _>("name").ok().as_deref() == Some("id"))
    {
        return Ok(quote_identifier("id"));
    }

    Ok("rowid".to_string())
}

fn encode_row(row: &SqliteRow) -> AppResult<JsonValue> {
    let mut object = JsonMap::new();
    for (index, column) in row.columns().iter().enumerate() {
        let raw = row.try_get_raw(index).map_err(|error| {
            AppError::Repository(format!(
                "failed to read backup column {} from row: {error}",
                column.name()
            ))
        })?;

        let value = if raw.is_null() {
            JsonValue::Null
        } else {
            match raw.type_info().name() {
                "INTEGER" => JsonValue::from(row.try_get::<i64, _>(index).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to decode integer column {}: {error}",
                        column.name()
                    ))
                })?),
                "REAL" => {
                    let value = row.try_get::<f64, _>(index).map_err(|error| {
                        AppError::Repository(format!(
                            "failed to decode real column {}: {error}",
                            column.name()
                        ))
                    })?;
                    JsonValue::from(value)
                }
                "BLOB" => {
                    encode_blob_value(&row.try_get::<Vec<u8>, _>(index).map_err(|error| {
                        AppError::Repository(format!(
                            "failed to decode blob column {}: {error}",
                            column.name()
                        ))
                    })?)
                }
                _ => JsonValue::String(row.try_get::<String, _>(index).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to decode text column {}: {error}",
                        column.name()
                    ))
                })?),
            }
        };

        object.insert(column.name().to_string(), value);
    }

    Ok(JsonValue::Object(object))
}

fn encode_blob_value(bytes: &[u8]) -> JsonValue {
    let mut object = JsonMap::new();
    object.insert(
        BLOB_MARKER_TYPE.to_string(),
        JsonValue::String("blob".to_string()),
    );
    object.insert(
        BLOB_MARKER_BASE64.to_string(),
        JsonValue::String(STANDARD.encode(bytes)),
    );
    JsonValue::Object(object)
}

async fn import_table_part(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    table: &str,
    part_path: &Path,
) -> AppResult<()> {
    let columns = table_columns(conn, table).await?;
    let insert_sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        quote_identifier(table),
        columns
            .iter()
            .map(|column| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", "),
        std::iter::repeat_n("?", columns.len())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let file = File::open(part_path).map_err(|error| {
        AppError::Validation(format!("backup table payload missing for {table}: {error}"))
    })?;
    let decoder = zstd::Decoder::new(BufReader::new(file)).map_err(|error| {
        AppError::Validation(format!(
            "backup table payload for {table} is invalid: {error}"
        ))
    })?;
    let reader = BufReader::new(decoder);

    for (line_number, line) in reader.lines().enumerate() {
        let line = line.map_err(|error| {
            AppError::Validation(format!(
                "failed to read backup row {table}:{line_number}: {error}"
            ))
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let value: JsonValue = serde_json::from_str(&line).map_err(|error| {
            AppError::Validation(format!(
                "invalid backup row for {table}:{line_number}: {error}"
            ))
        })?;
        let object = value.as_object().ok_or_else(|| {
            AppError::Validation(format!(
                "backup row for {table}:{line_number} is not an object"
            ))
        })?;

        let mut query = sqlx::query(&insert_sql);
        for column in &columns {
            let value = object.get(column).unwrap_or(&JsonValue::Null);
            query = bind_json_value(query, value)?;
        }
        query.execute(&mut **conn).await.map_err(|error| {
            AppError::Validation(format!(
                "failed to import backup row for {table}:{line_number}: {error}"
            ))
        })?;
    }

    Ok(())
}

async fn table_columns(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    table: &str,
) -> AppResult<Vec<String>> {
    let pragma = format!("PRAGMA table_info({})", quote_identifier(table));
    let rows = sqlx::query(&pragma)
        .fetch_all(&mut **conn)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to inspect table columns for {table}: {error}"
            ))
        })?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect())
}

fn bind_json_value<'q>(
    query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    value: &JsonValue,
) -> AppResult<sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>> {
    Ok(match value {
        JsonValue::Null => query.bind(None::<String>),
        JsonValue::Bool(value) => query.bind(if *value { 1_i64 } else { 0_i64 }),
        JsonValue::Number(value) => {
            if let Some(value) = value.as_i64() {
                query.bind(value)
            } else if let Some(value) = value.as_u64() {
                let value = i64::try_from(value).map_err(|_| {
                    AppError::Validation(
                        "backup row contains an integer outside SQLite i64 range".into(),
                    )
                })?;
                query.bind(value)
            } else if let Some(value) = value.as_f64() {
                query.bind(value)
            } else {
                return Err(AppError::Validation(
                    "backup row contains an unsupported numeric value".into(),
                ));
            }
        }
        JsonValue::String(value) => query.bind(value.clone()),
        JsonValue::Object(object)
            if object.get(BLOB_MARKER_TYPE).and_then(JsonValue::as_str) == Some("blob") =>
        {
            let encoded = object
                .get(BLOB_MARKER_BASE64)
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    AppError::Validation("backup blob payload is missing base64 bytes".into())
                })?;
            let bytes = STANDARD.decode(encoded).map_err(|error| {
                AppError::Validation(format!("backup blob payload is invalid base64: {error}"))
            })?;
            query.bind(bytes)
        }
        _ => {
            return Err(AppError::Validation(
                "backup row contains an unsupported JSON value".into(),
            ));
        }
    })
}

fn quote_identifier(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
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
