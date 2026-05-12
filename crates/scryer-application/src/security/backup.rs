use super::backup_bundle::{
    BACKUP_ENCRYPTED_EXTENSION, BACKUP_FORMAT_VERSION, BACKUP_PLAINTEXT_EXTENSION,
    BackupBundleExportRequest, BackupExportSecrets,
};
use super::*;
use crate::types::BackupStatus;
use scryer_domain::ConfigurationChangeAction;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::{error, info, warn};

const BACKUP_METADATA_EXTENSION: &str = ".metadata.json";

fn metadata_filename(filename: &str) -> String {
    format!("{filename}{BACKUP_METADATA_EXTENSION}")
}

fn metadata_path(backup_dir: &Path, filename: &str) -> PathBuf {
    backup_dir.join(metadata_filename(filename))
}

fn bundle_path(backup_dir: &Path, filename: &str) -> PathBuf {
    backup_dir.join(filename)
}

fn is_supported_backup_filename(filename: &str) -> bool {
    filename.starts_with("scryer_backup_")
        && !filename.contains('/')
        && !filename.contains('\\')
        && (filename.ends_with(BACKUP_PLAINTEXT_EXTENSION)
            || filename.ends_with(BACKUP_ENCRYPTED_EXTENSION))
}

fn build_backup_filename(encrypted: bool) -> String {
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
    let extension = if encrypted {
        BACKUP_ENCRYPTED_EXTENSION
    } else {
        BACKUP_PLAINTEXT_EXTENSION
    };
    format!("scryer_backup_{timestamp}{extension}")
}

fn creating_backup_info(
    filename: String,
    created_at: String,
    source_engine: String,
    source_migration_key: Option<String>,
    encrypted: bool,
) -> BackupInfo {
    BackupInfo {
        filename,
        size_bytes: 0,
        created_at,
        format_version: BACKUP_FORMAT_VERSION.to_string(),
        source_engine,
        source_migration_key,
        encrypted,
        row_counts: BTreeMap::new(),
        status: BackupStatus::Creating,
        error_message: None,
    }
}

fn failed_backup_info(base: BackupInfo, error_message: String) -> BackupInfo {
    BackupInfo {
        status: BackupStatus::Failed,
        error_message: Some(error_message),
        ..base
    }
}

fn normalize_backup_info(mut info: BackupInfo, backup_dir: &Path) -> BackupInfo {
    let path = bundle_path(backup_dir, &info.filename);
    match info.status {
        BackupStatus::Ready => match std::fs::metadata(&path) {
            Ok(metadata) => {
                info.size_bytes = metadata.len();
            }
            Err(_) => {
                info.size_bytes = 0;
                info.status = BackupStatus::Failed;
                if info.error_message.is_none() {
                    info.error_message = Some("backup bundle file is missing".to_string());
                }
            }
        },
        BackupStatus::Creating | BackupStatus::Failed => {
            info.size_bytes = std::fs::metadata(&path)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
        }
    }
    info
}

fn list_backup_files(backup_dir: &Path) -> Vec<BackupInfo> {
    let mut entries = Vec::new();
    let Ok(read_dir) = std::fs::read_dir(backup_dir) else {
        return entries;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(bundle_filename) = filename.strip_suffix(BACKUP_METADATA_EXTENSION) else {
            continue;
        };
        if !is_supported_backup_filename(bundle_filename) {
            continue;
        }

        match std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<BackupInfo>(&bytes).ok())
        {
            Some(info) => entries.push(normalize_backup_info(info, backup_dir)),
            None => warn!(path = %path.display(), "failed to load backup metadata entry"),
        }
    }

    entries.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.filename.cmp(&a.filename))
    });
    entries
}

fn write_backup_metadata(backup_dir: &Path, info: &BackupInfo) -> AppResult<()> {
    let path = metadata_path(backup_dir, &info.filename);
    let temp_path = path.with_extension("metadata.json.tmp");
    let payload = serde_json::to_vec_pretty(info).map_err(|error| {
        AppError::Repository(format!("failed to encode backup metadata: {error}"))
    })?;

    std::fs::write(&temp_path, payload).map_err(|error| {
        AppError::Repository(format!("failed to write backup metadata: {error}"))
    })?;
    ensure_owner_only_permissions(&temp_path)?;
    std::fs::rename(&temp_path, &path).map_err(|error| {
        AppError::Repository(format!("failed to finalize backup metadata: {error}"))
    })?;
    ensure_owner_only_permissions(&path)?;
    Ok(())
}

fn remove_backup_artifacts(backup_dir: &Path, filename: &str) -> AppResult<bool> {
    let bundle = bundle_path(backup_dir, filename);
    let metadata = metadata_path(backup_dir, filename);
    let bundle_exists = bundle.exists();
    let metadata_exists = metadata.exists();
    if !bundle_exists && !metadata_exists {
        return Ok(false);
    }

    if bundle_exists {
        std::fs::remove_file(&bundle).map_err(|error| {
            AppError::Repository(format!("failed to delete backup bundle: {error}"))
        })?;
    }
    if metadata_exists {
        std::fs::remove_file(&metadata).map_err(|error| {
            AppError::Repository(format!("failed to delete backup metadata: {error}"))
        })?;
    }

    Ok(true)
}

#[cfg(unix)]
fn ensure_owner_only_permissions(path: &Path) -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;

    if !path.exists() {
        return Ok(());
    }

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|error| {
        AppError::Repository(format!("failed to set backup permissions: {error}"))
    })?;
    Ok(())
}

#[cfg(not(unix))]
fn ensure_owner_only_permissions(_path: &Path) -> AppResult<()> {
    Ok(())
}

async fn export_backup_file(
    exporter: Arc<dyn LogicalBackupExporter>,
    backup_dir: &Path,
    filename: &str,
    passphrase: Option<&str>,
    source_engine: String,
    source_migration_key: Option<String>,
    secrets: BackupExportSecrets,
) -> AppResult<BackupInfo> {
    let output_path = bundle_path(backup_dir, filename);
    let outcome = exporter
        .export_backup_bundle(BackupBundleExportRequest {
            output_path: output_path.clone(),
            passphrase: passphrase.map(str::to_string),
            source_migration_key,
            source_scryer_version: env!("CARGO_PKG_VERSION").to_string(),
            source_engine,
            secrets,
        })
        .await?;

    let size_bytes = std::fs::metadata(&output_path)
        .map_err(|error| AppError::Repository(format!("failed to stat backup bundle: {error}")))?
        .len();
    let summary = outcome.summary;

    Ok(BackupInfo {
        filename: filename.to_string(),
        size_bytes,
        created_at: summary.created_at,
        format_version: summary.format_version,
        source_engine: summary.source_engine,
        source_migration_key: summary.source_migration_key,
        encrypted: summary.encrypted,
        row_counts: summary.row_counts,
        status: BackupStatus::Ready,
        error_message: None,
    })
}

pub trait BackupService {
    fn backup_dir(&self) -> PathBuf;
}

impl BackupService for AppUseCase {
    fn backup_dir(&self) -> PathBuf {
        self.services.config.backup_dir.clone()
    }
}

impl AppUseCase {
    async fn collect_backup_export_secrets(&self) -> AppResult<BackupExportSecrets> {
        let encryption_master_key = self
            .services
            .config
            .system_info
            .current_encryption_key_base64()
            .await?
            .ok_or_else(|| {
                AppError::Validation(
                    "backup export requires a configured encryption master key".into(),
                )
            })?;

        Ok(BackupExportSecrets {
            encryption_master_key,
            jwt_signing_secret: self.auth.jwt_signing_salt.clone(),
            smg_registration_secret: self.services.config.smg_registration_secret.clone(),
            smg_ca_cert: self.services.config.smg_ca_cert.clone(),
            smg_gateway_url: self.services.config.smg_gateway_url.clone(),
        })
    }

    pub async fn create_backup(
        &self,
        actor: &User,
        passphrase: Option<&str>,
    ) -> AppResult<BackupInfo> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let dir = self.backup_dir();
        std::fs::create_dir_all(&dir).map_err(|error| {
            AppError::Repository(format!("failed to create backup directory: {error}"))
        })?;

        let datastore_info = self.services.config.system_info.datastore_info().await?;
        let source_migration_key = datastore_info.current_migration_key.clone();
        let secrets = self.collect_backup_export_secrets().await?;
        let queued = creating_backup_info(
            build_backup_filename(passphrase.is_some()),
            chrono::Utc::now().to_rfc3339(),
            datastore_info.engine.clone(),
            source_migration_key.clone(),
            passphrase.is_some(),
        );
        write_backup_metadata(&dir, &queued)?;

        let filename = queued.filename.clone();
        let app = self.clone();
        let actor_id = actor.id.clone();
        let exporter = self.services.config.logical_backup_exporter.clone();
        let queued_for_task = queued.clone();
        let dir_for_task = dir.clone();
        let passphrase_for_task = passphrase.map(str::to_string);
        let source_engine = datastore_info.engine;

        info!(filename = %filename, encrypted = queued.encrypted, "backup bundle scheduled");

        tokio::spawn(async move {
            let result = export_backup_file(
                exporter,
                &dir_for_task,
                &filename,
                passphrase_for_task.as_deref(),
                source_engine,
                source_migration_key,
                secrets,
            )
            .await;

            let next_info = match result {
                Ok(info) => {
                    info!(
                        filename = %info.filename,
                        size_bytes = info.size_bytes,
                        encrypted = info.encrypted,
                        "backup bundle created"
                    );
                    info
                }
                Err(error) => {
                    let message = error.to_string();
                    let _ = std::fs::remove_file(bundle_path(&dir_for_task, &filename));
                    error!(
                        filename = %filename,
                        error = %message,
                        "backup bundle creation failed"
                    );
                    failed_backup_info(queued_for_task, message)
                }
            };

            if let Err(error) = write_backup_metadata(&dir_for_task, &next_info) {
                error!(
                    filename = %filename,
                    error = %error,
                    "failed to persist backup bundle metadata"
                );
            }

            app.emit_configuration_changed_event(
                Some(actor_id),
                "backup",
                Some(filename),
                ConfigurationChangeAction::Saved,
            )
            .await;
        });

        Ok(queued)
    }

    pub async fn list_backups(&self, actor: &User) -> AppResult<Vec<BackupInfo>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        Ok(list_backup_files(&self.backup_dir()))
    }

    pub async fn delete_backup(&self, actor: &User, filename: &str) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        if !is_supported_backup_filename(filename) {
            return Err(AppError::Validation("invalid backup filename".into()));
        }

        let deleted = remove_backup_artifacts(&self.backup_dir(), filename)?;
        if !deleted {
            return Ok(false);
        }

        info!(filename, "backup deleted");
        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "backup",
            Some(filename.to_string()),
            ConfigurationChangeAction::Deleted,
        )
        .await;
        Ok(true)
    }

    /// Enforce backup retention: delete oldest ready backups exceeding the retention count.
    pub async fn enforce_backup_retention(&self, retention_count: usize) -> AppResult<u32> {
        let dir = self.backup_dir();
        let entries = list_backup_files(&dir);
        let ready_entries = entries
            .into_iter()
            .filter(|entry| entry.status == BackupStatus::Ready)
            .collect::<Vec<_>>();
        let mut deleted = 0u32;

        if ready_entries.len() > retention_count {
            for entry in &ready_entries[retention_count..] {
                match remove_backup_artifacts(&dir, &entry.filename) {
                    Ok(true) => deleted += 1,
                    Ok(false) => {}
                    Err(error) => warn!(
                        filename = %entry.filename,
                        error = %error,
                        "failed to remove old backup"
                    ),
                }
            }
        }

        if deleted > 0 {
            info!(deleted, "old backups pruned by retention policy");
        }
        Ok(deleted)
    }

    /// Auto-backup if enough time has passed since the last completed backup.
    pub async fn auto_backup_if_due(&self) -> AppResult<()> {
        let interval_hours: u64 = self
            .read_setting_string_value_for_scope(
                SETTINGS_SCOPE_SYSTEM,
                "backup.interval_hours",
                None,
            )
            .await?
            .and_then(|value| value.parse().ok())
            .unwrap_or(24);

        if interval_hours == 0 {
            return Ok(());
        }

        let retention_count: usize = self
            .read_setting_string_value_for_scope(
                SETTINGS_SCOPE_SYSTEM,
                "backup.retention_count",
                None,
            )
            .await?
            .and_then(|value| value.parse().ok())
            .unwrap_or(7);

        let dir = self.backup_dir();
        let entries = list_backup_files(&dir);
        if entries
            .iter()
            .any(|entry| entry.status == BackupStatus::Creating)
        {
            return Ok(());
        }

        let newest_ready = entries
            .iter()
            .find(|entry| entry.status == BackupStatus::Ready);
        let needs_backup = if let Some(newest) = newest_ready {
            if let Ok(last_time) = chrono::DateTime::parse_from_rfc3339(&newest.created_at) {
                let elapsed = chrono::Utc::now() - last_time.with_timezone(&chrono::Utc);
                elapsed > chrono::Duration::hours(interval_hours as i64)
            } else {
                true
            }
        } else {
            true
        };

        if needs_backup {
            let actor = self.find_or_create_default_user().await?;
            std::fs::create_dir_all(&dir).map_err(|error| {
                AppError::Repository(format!("failed to create backup directory: {error}"))
            })?;

            let filename = build_backup_filename(false);
            let datastore_info = self.services.config.system_info.datastore_info().await?;
            let info = export_backup_file(
                self.services.config.logical_backup_exporter.clone(),
                &dir,
                &filename,
                None,
                datastore_info.engine,
                datastore_info.current_migration_key,
                self.collect_backup_export_secrets().await?,
            )
            .await?;
            write_backup_metadata(&dir, &info)?;
            self.emit_configuration_changed_event(
                Some(actor.id.clone()),
                "backup",
                Some(info.filename.clone()),
                ConfigurationChangeAction::Saved,
            )
            .await;
            self.enforce_backup_retention(retention_count).await?;
        }

        Ok(())
    }
}
