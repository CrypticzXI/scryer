use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::context::{
    app_from_ctx, require_app_permission, require_config_step_up, restore_context_from_ctx,
    to_gql_error,
};
use crate::mappers::from_backup_info;
use crate::types::{BackupInfoPayload, BackupRowCountPayload};
use async_graphql::{Context, Error, Object, Result as GqlResult, SimpleObject, Upload};
use scryer_application::{
    AppError, AppUseCase, BackupBundleInspectSummary, inspect_backup_bundle,
    prepare_backup_restore_payload,
};
use scryer_domain::AppPermission;
use scryer_interface_core::{
    RestoreDatastoreConfig, RestoreDatastoreEngine, RestoreDatastoreHandle,
    RestoreSqliteDatastoreRequest,
};

const INSTANCE_SECRETS_ENV_FILENAME: &str = "instance-secrets.env";
const PENDING_RESTORE_DB_FILENAME: &str = "restored-scryer.db";
const PENDING_RESTORE_PREPARED_BUNDLE_DIRNAME: &str = "prepared-bundle";
const PENDING_RESTORE_DIRNAME: &str = "restore-pending";
const PENDING_RESTORE_READY_FILENAME: &str = "restore-ready";
const RESTORE_STAGING_DIRNAME: &str = "restore-staging";
const RESTORE_UPLOAD_TTL_SECONDS: u64 = 24 * 60 * 60;
const STAGED_BUNDLE_FILENAME: &str = "bundle.upload";

#[derive(Clone, SimpleObject)]
struct RestoreSummaryPayload {
    format_version: String,
    created_at: String,
    source_scryer_version: String,
    source_engine: String,
    source_migration_key: Option<String>,
    encrypted: bool,
    row_counts: Vec<BackupRowCountPayload>,
    total_rows: String,
}

#[derive(Clone, SimpleObject)]
struct RestoreInspectPayload {
    upload_id: String,
    summary: RestoreSummaryPayload,
}

#[derive(Clone, SimpleObject)]
struct RestoreApplyPayload {
    summary: RestoreSummaryPayload,
}

#[derive(Clone, SimpleObject)]
struct BackupDownloadUrlPayload {
    download_url: String,
    download_authorization_token: String,
    expires_at: String,
}

#[derive(Default)]
pub struct BackupMutations;

#[Object]
impl BackupMutations {
    async fn create_backup(
        &self,
        ctx: &Context<'_>,
        password: String,
    ) -> GqlResult<BackupInfoPayload> {
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        let actor = require_config_step_up(ctx).await?;
        let info = app
            .create_backup(&actor, &password)
            .await
            .map_err(to_gql_error)?;
        Ok(from_backup_info(info))
    }

    async fn prepare_backup_download(
        &self,
        ctx: &Context<'_>,
        filename: String,
    ) -> GqlResult<BackupDownloadUrlPayload> {
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let actor = require_config_step_up(ctx).await?;
        let app = app_from_ctx(ctx)?;
        let ticket = app
            .prepare_backup_download(&actor, &filename)
            .await
            .map_err(to_gql_error)?;
        let encoded_filename = encode_path_segment(&filename);

        Ok(BackupDownloadUrlPayload {
            download_url: format!("/backups/{encoded_filename}/download"),
            download_authorization_token: ticket.token,
            expires_at: ticket.expires_at,
        })
    }

    async fn delete_backup(&self, ctx: &Context<'_>, filename: String) -> GqlResult<bool> {
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        let actor = require_config_step_up(ctx).await?;
        app.delete_backup(&actor, &filename)
            .await
            .map_err(to_gql_error)
    }

    async fn inspect_restore_bundle(
        &self,
        ctx: &Context<'_>,
        bundle_upload: Upload,
        password: Option<String>,
    ) -> GqlResult<RestoreInspectPayload> {
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        ensure_setup_mode(&app).await.map_err(to_gql_error)?;

        let restore = restore_context_from_ctx(ctx)?;
        ensure_restore_supported(&restore.datastore_config).map_err(to_gql_error)?;

        let upload = bundle_upload
            .value(ctx)
            .map_err(|error| Error::new(format!("invalid upload: {error}")))?;
        let password = normalize_password(password);

        prune_stale_restore_uploads(&restore.data_dir);
        let upload_id = next_restore_upload_id();
        let upload_dir = restore_upload_dir(&restore.data_dir, &upload_id);
        tokio::fs::create_dir_all(&upload_dir)
            .await
            .map_err(|error| {
                to_gql_error(AppError::Repository(format!(
                    "failed to create restore staging directory: {error}"
                )))
            })?;
        ensure_owner_only_dir_permissions(&upload_dir).map_err(|error| {
            to_gql_error(AppError::Repository(format!(
                "failed to protect restore staging directory: {error}"
            )))
        })?;

        let bundle_path = upload_dir.join(STAGED_BUNDLE_FILENAME);
        if let Err(error) = write_uploaded_bundle(upload, bundle_path.clone()).await {
            let _ = tokio::fs::remove_dir_all(&upload_dir).await;
            return Err(to_gql_error(error));
        }

        let inspect_password = password.clone();
        let inspect_path = bundle_path.clone();
        let summary = match tokio::task::spawn_blocking(move || {
            inspect_backup_bundle(&inspect_path, inspect_password.as_deref())
        })
        .await
        {
            Ok(Ok(summary)) => summary,
            Ok(Err(error)) => {
                let _ = tokio::fs::remove_dir_all(&upload_dir).await;
                return Err(to_gql_error(error));
            }
            Err(error) => {
                let _ = tokio::fs::remove_dir_all(&upload_dir).await;
                return Err(to_gql_error(AppError::Repository(format!(
                    "restore inspect worker failed: {error}"
                ))));
            }
        };

        Ok(RestoreInspectPayload {
            upload_id,
            summary: restore_summary_payload(&summary),
        })
    }

    async fn apply_restore_bundle(
        &self,
        ctx: &Context<'_>,
        upload_id: String,
        password: Option<String>,
    ) -> GqlResult<RestoreApplyPayload> {
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        ensure_setup_mode(&app).await.map_err(to_gql_error)?;

        let restore = restore_context_from_ctx(ctx)?;
        ensure_restore_supported(&restore.datastore_config).map_err(to_gql_error)?;

        let upload_id = upload_id.trim();
        if upload_id.is_empty() || upload_id.contains('/') || upload_id.contains('\\') {
            return Err(to_gql_error(AppError::Validation(
                "invalid restore upload id".into(),
            )));
        }

        let bundle_path = restore_upload_bundle_path(&restore.data_dir, upload_id);
        if !bundle_path.exists() {
            return Err(to_gql_error(AppError::NotFound(
                "restore upload could not be found; inspect the bundle again".into(),
            )));
        }

        let data_dir = restore.data_dir.clone();
        let datastore_config = restore.datastore_config;
        let datastore = restore.datastore.clone();
        let password = normalize_password(password);
        let summary = match tokio::task::spawn_blocking(move || {
            stage_restore_bundle(data_dir, bundle_path, datastore_config, datastore, password)
        })
        .await
        {
            Ok(Ok(summary)) => summary,
            Ok(Err(error)) => return Err(to_gql_error(error)),
            Err(error) => {
                return Err(to_gql_error(AppError::Repository(format!(
                    "restore worker failed: {error}"
                ))));
            }
        };

        let _ = tokio::fs::remove_dir_all(restore_upload_dir(&restore.data_dir, upload_id)).await;
        finish_restore_apply_result(&restore, Ok(summary)).map_err(to_gql_error)
    }
}

fn normalize_password(password: Option<String>) -> Option<String> {
    password
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn finish_restore_apply(
    restore: &crate::context::RestoreContext,
    summary: RestoreSummaryPayload,
) -> Result<RestoreApplyPayload, AppError> {
    restore.restart.schedule_restart();
    Ok(RestoreApplyPayload { summary })
}

fn finish_restore_apply_result(
    restore: &crate::context::RestoreContext,
    result: Result<RestoreSummaryPayload, AppError>,
) -> Result<RestoreApplyPayload, AppError> {
    match result {
        Ok(summary) => finish_restore_apply(restore, summary),
        Err(error) => Err(error),
    }
}

fn restore_upload_dir(data_dir: &Path, upload_id: &str) -> PathBuf {
    data_dir.join(RESTORE_STAGING_DIRNAME).join(upload_id)
}

fn restore_upload_bundle_path(data_dir: &Path, upload_id: &str) -> PathBuf {
    restore_upload_dir(data_dir, upload_id).join(STAGED_BUNDLE_FILENAME)
}

fn pending_restore_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(PENDING_RESTORE_DIRNAME)
}

fn pending_restore_db_path(data_dir: &Path) -> PathBuf {
    pending_restore_dir(data_dir).join(PENDING_RESTORE_DB_FILENAME)
}

fn pending_restore_prepared_bundle_dir(data_dir: &Path) -> PathBuf {
    pending_restore_dir(data_dir).join(PENDING_RESTORE_PREPARED_BUNDLE_DIRNAME)
}

fn pending_restore_instance_secrets_path(data_dir: &Path) -> PathBuf {
    pending_restore_dir(data_dir).join(INSTANCE_SECRETS_ENV_FILENAME)
}

fn pending_restore_ready_path(data_dir: &Path) -> PathBuf {
    pending_restore_dir(data_dir).join(PENDING_RESTORE_READY_FILENAME)
}

async fn ensure_setup_mode(app: &AppUseCase) -> Result<(), AppError> {
    if app.setup_complete().await? {
        return Err(AppError::Validation(
            "restore is only available while setup is still incomplete".into(),
        ));
    }
    Ok(())
}

fn ensure_restore_supported(_datastore_config: &RestoreDatastoreConfig) -> Result<(), AppError> {
    Ok(())
}

fn restore_summary_payload(summary: &BackupBundleInspectSummary) -> RestoreSummaryPayload {
    RestoreSummaryPayload {
        format_version: summary.format_version.clone(),
        created_at: summary.created_at.clone(),
        source_scryer_version: summary.source_scryer_version.clone(),
        source_engine: summary.source_engine.clone(),
        source_migration_key: summary.source_migration_key.clone(),
        encrypted: summary.encrypted,
        row_counts: summary
            .row_counts
            .iter()
            .map(|(table, row_count)| BackupRowCountPayload {
                table: table.clone(),
                row_count: row_count.to_string(),
            })
            .collect(),
        total_rows: summary.total_rows().to_string(),
    }
}

fn prune_stale_restore_uploads(data_dir: &Path) {
    let uploads_dir = data_dir.join(RESTORE_STAGING_DIRNAME);
    let Ok(entries) = std::fs::read_dir(&uploads_dir) else {
        return;
    };
    let cutoff = SystemTime::now().checked_sub(Duration::from_secs(RESTORE_UPLOAD_TTL_SECONDS));

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let expired = metadata
            .modified()
            .ok()
            .zip(cutoff)
            .is_some_and(|(modified, cutoff)| modified < cutoff);
        if expired {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

fn next_restore_upload_id() -> String {
    let timestamp_micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros())
        .unwrap_or(0);
    format!("restore_{}_{}", timestamp_micros, std::process::id())
}

async fn write_uploaded_bundle(
    upload: async_graphql::UploadValue,
    bundle_path: PathBuf,
) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let mut output = std::fs::File::create(&bundle_path).map_err(|error| {
            AppError::Repository(format!("failed to create restore staging bundle: {error}"))
        })?;
        let mut reader = upload.into_read();
        std::io::copy(&mut reader, &mut output).map_err(|error| {
            AppError::Repository(format!("failed to write restore staging bundle: {error}"))
        })?;
        output.flush().map_err(|error| {
            AppError::Repository(format!("failed to flush restore staging bundle: {error}"))
        })?;
        ensure_owner_only_permissions(&bundle_path).map_err(|error| {
            AppError::Repository(format!("failed to protect restore staging bundle: {error}"))
        })?;
        Ok(())
    })
    .await
    .map_err(|error| AppError::Repository(format!("restore upload worker failed: {error}")))?
}

fn stage_restore_bundle(
    data_dir: PathBuf,
    bundle_path: PathBuf,
    datastore_config: RestoreDatastoreConfig,
    datastore: RestoreDatastoreHandle,
    password: Option<String>,
) -> Result<RestoreSummaryPayload, AppError> {
    let pending_dir = pending_restore_dir(&data_dir);
    let pending_db_path = pending_restore_db_path(&data_dir);
    let pending_prepared_bundle_dir = pending_restore_prepared_bundle_dir(&data_dir);
    let pending_secrets_path = pending_restore_instance_secrets_path(&data_dir);
    let pending_ready_path = pending_restore_ready_path(&data_dir);
    let _ = std::fs::remove_dir_all(&pending_dir);
    std::fs::create_dir_all(&pending_dir).map_err(|error| {
        AppError::Repository(format!(
            "failed to create pending restore directory: {error}"
        ))
    })?;
    ensure_owner_only_dir_permissions(&pending_dir).map_err(|error| {
        AppError::Repository(format!(
            "failed to protect pending restore directory: {error}"
        ))
    })?;

    let result = (|| match datastore_config.engine {
        RestoreDatastoreEngine::Postgres => {
            let prepared = prepare_backup_restore_payload(&bundle_path, password.as_deref())?;
            let summary = restore_summary_payload(&prepared.summary());
            let instance_secrets_env = prepared.instance_secrets_env()?;
            write_owner_only_file_atomically(&pending_secrets_path, &instance_secrets_env)?;
            prepared.persist_extracted_dir(&pending_prepared_bundle_dir)?;
            Ok::<_, AppError>(summary)
        }
        RestoreDatastoreEngine::Sqlite => {
            let prepared =
                datastore.restore_sqlite_bundle_to_path(RestoreSqliteDatastoreRequest {
                    target_db_path: pending_db_path.clone(),
                    migration_mode: datastore_config.migration_mode,
                    bundle_path: bundle_path.clone(),
                    passphrase: password.clone(),
                })?;

            write_owner_only_file_atomically(
                &pending_secrets_path,
                &prepared.instance_secrets_env(),
            )?;

            let summary = restore_summary_payload(prepared.summary());
            ensure_owner_only_permissions(&pending_db_path).map_err(|error| {
                AppError::Repository(format!(
                    "failed to protect pending restore database: {error}"
                ))
            })?;
            Ok::<_, AppError>(summary)
        }
    })();

    match result {
        Ok(summary) => {
            let marker_result = std::fs::write(&pending_ready_path, b"ready")
                .and_then(|_| ensure_owner_only_permissions(&pending_ready_path));
            if let Err(error) = marker_result {
                let _ = std::fs::remove_dir_all(&pending_dir);
                return Err(AppError::Repository(format!(
                    "failed to mark pending restore ready: {error}"
                )));
            }
            Ok(summary)
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&pending_dir);
            Err(error)
        }
    }
}

#[cfg(unix)]
fn ensure_owner_only_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if !path.exists() {
        return Ok(());
    }

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn ensure_owner_only_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn ensure_owner_only_dir_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if !path.exists() {
        return Ok(());
    }

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn ensure_owner_only_dir_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

fn write_owner_only_file_atomically(path: &Path, contents: &str) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| {
        AppError::Repository(format!(
            "cannot resolve parent directory for {}",
            path.display()
        ))
    })?;
    std::fs::create_dir_all(parent).map_err(|error| {
        AppError::Repository(format!(
            "failed to create parent directory for {}: {error}",
            path.display()
        ))
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            AppError::Repository(format!("invalid restore secrets path {}", path.display()))
        })?;
    let temp_path = parent.join(format!(
        ".{file_name}.tmp.{}.{}",
        std::process::id(),
        next_restore_upload_id()
    ));

    let write_result = (|| -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp_path)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(AppError::Repository(format!(
            "failed to write restore secrets {}: {error}",
            temp_path.display()
        )));
    }

    std::fs::rename(&temp_path, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp_path);
        AppError::Repository(format!(
            "failed to move restore secrets into place {}: {error}",
            path.display()
        ))
    })?;
    ensure_owner_only_permissions(path).map_err(|error| {
        AppError::Repository(format!(
            "failed to protect restore secrets {}: {error}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RestoreRestartHandle;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn test_restore_summary_payload() -> RestoreSummaryPayload {
        RestoreSummaryPayload {
            format_version: "scryer-backup-bundle-v1".to_string(),
            created_at: "2026-05-14T00:00:00Z".to_string(),
            source_scryer_version: "0.15.0".to_string(),
            source_engine: "sqlite".to_string(),
            source_migration_key: Some("0112".to_string()),
            encrypted: true,
            row_counts: vec![],
            total_rows: "0".to_string(),
        }
    }

    #[test]
    fn finish_restore_apply_schedules_restart_on_success() {
        let restart_calls = Arc::new(AtomicUsize::new(0));
        let restart_calls_handle = restart_calls.clone();
        let restore = crate::context::RestoreContext {
            data_dir: PathBuf::from("/tmp/scryer"),
            datastore_config: RestoreDatastoreConfig {
                engine: RestoreDatastoreEngine::Sqlite,
                migration_mode: scryer_interface_core::RestoreMigrationMode::Apply,
            },
            datastore: RestoreDatastoreHandle::unavailable(),
            restart: RestoreRestartHandle::new(move || {
                restart_calls_handle.fetch_add(1, Ordering::SeqCst);
            }),
        };

        let payload =
            finish_restore_apply(&restore, test_restore_summary_payload()).expect("finish restore");

        assert_eq!(payload.summary.total_rows, "0");
        assert_eq!(restart_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn finish_restore_apply_result_does_not_schedule_restart_on_error() {
        let restart_calls = Arc::new(AtomicUsize::new(0));
        let restart_calls_handle = restart_calls.clone();
        let restore = crate::context::RestoreContext {
            data_dir: PathBuf::from("/tmp/scryer"),
            datastore_config: RestoreDatastoreConfig {
                engine: RestoreDatastoreEngine::Sqlite,
                migration_mode: scryer_interface_core::RestoreMigrationMode::Apply,
            },
            datastore: RestoreDatastoreHandle::unavailable(),
            restart: RestoreRestartHandle::new(move || {
                restart_calls_handle.fetch_add(1, Ordering::SeqCst);
            }),
        };

        let result =
            finish_restore_apply_result(&restore, Err(AppError::Validation("boom".to_string())));

        assert!(result.is_err());
        assert_eq!(restart_calls.load(Ordering::SeqCst), 0);
    }
}
