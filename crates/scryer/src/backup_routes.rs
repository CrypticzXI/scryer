use std::collections::BTreeMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, Multipart, Path as AxumPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use scryer_application::{
    AppError, AppUseCase, BackupInfo, BackupService, BackupStatus, inspect_backup_bundle,
};
use scryer_domain::AppPermission;
use scryer_infrastructure::{
    DatastoreConfig, DatastoreEngine, datastore_file_path, restore_backup_bundle_to_datastore,
    restore_backup_bundle_to_datastore_path,
};
use scryer_interface::context::AuthRuntimeStateHandle;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use crate::middleware::{map_app_error, resolve_actor_with_app_permission};

const BACKUP_METADATA_EXTENSION: &str = ".metadata.json";
const INSTANCE_SECRETS_ENV_FILENAME: &str = "instance-secrets.env";
const PENDING_RESTORE_DB_FILENAME: &str = "restored-scryer.db";
const PENDING_RESTORE_DIRNAME: &str = "restore-pending";
const PENDING_RESTORE_READY_FILENAME: &str = "restore-ready";
const RESTORE_STAGING_DIRNAME: &str = "restore-staging";
const RESTORE_UPLOAD_TTL_SECONDS: u64 = 24 * 60 * 60;
const STAGED_BUNDLE_FILENAME: &str = "bundle.upload";

#[derive(Clone)]
pub(crate) struct BackupRouteState {
    pub(crate) app: AppUseCase,
    pub(crate) auth_runtime: AuthRuntimeStateHandle,
    pub(crate) data_dir: PathBuf,
    pub(crate) datastore_engine: DatastoreEngine,
    pub(crate) datastore_config: DatastoreConfig,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Serialize)]
pub(crate) struct RestoreSummaryResponse {
    pub format_version: String,
    pub created_at: String,
    pub source_scryer_version: String,
    pub source_engine: String,
    pub source_migration_key: Option<String>,
    pub encrypted: bool,
    pub row_counts: BTreeMap<String, u64>,
    pub total_rows: u64,
}

#[derive(Serialize)]
pub(crate) struct RestoreInspectResponse {
    pub upload_id: String,
    pub summary: RestoreSummaryResponse,
}

#[derive(Serialize)]
pub(crate) struct RestoreApplyResponse {
    pub summary: RestoreSummaryResponse,
}

#[derive(Deserialize)]
pub(crate) struct RestoreApplyRequest {
    pub upload_id: String,
    pub password: Option<String>,
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

fn pending_restore_instance_secrets_path(data_dir: &Path) -> PathBuf {
    pending_restore_dir(data_dir).join(INSTANCE_SECRETS_ENV_FILENAME)
}

fn pending_restore_ready_path(data_dir: &Path) -> PathBuf {
    pending_restore_dir(data_dir).join(PENDING_RESTORE_READY_FILENAME)
}

fn managed_instance_secrets_path(data_dir: &Path) -> PathBuf {
    data_dir.join(INSTANCE_SECRETS_ENV_FILENAME)
}

fn metadata_path_for_backup(backup_dir: &Path, filename: &str) -> PathBuf {
    backup_dir.join(format!("{filename}{BACKUP_METADATA_EXTENSION}"))
}

fn is_supported_backup_filename(filename: &str) -> bool {
    filename.starts_with("scryer_backup_")
        && !filename.contains('/')
        && !filename.contains('\\')
        && (filename.ends_with(".scryer-backup.tar.zst")
            || filename.ends_with(".scryer-backup.enc"))
}

fn normalize_password(password: Option<String>) -> Option<String> {
    password
}

fn next_restore_upload_id() -> String {
    let timestamp_micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros())
        .unwrap_or(0);
    format!("restore_{}_{}", timestamp_micros, std::process::id())
}

fn restore_summary_response(
    summary: &scryer_application::BackupBundleInspectSummary,
) -> RestoreSummaryResponse {
    RestoreSummaryResponse {
        format_version: summary.format_version.clone(),
        created_at: summary.created_at.clone(),
        source_scryer_version: summary.source_scryer_version.clone(),
        source_engine: summary.source_engine.clone(),
        source_migration_key: summary.source_migration_key.clone(),
        encrypted: summary.encrypted,
        row_counts: summary.row_counts.clone(),
        total_rows: summary.total_rows(),
    }
}

fn conflict_response(message: impl Into<String>) -> Response {
    (
        StatusCode::CONFLICT,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
        .into_response()
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
        drop(file);
        ensure_owner_only_permissions(&temp_path)?;
        std::fs::rename(&temp_path, path)?;
        ensure_owner_only_permissions(path)
    })();

    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(AppError::Repository(format!(
            "failed to write restore secrets atomically: {error}"
        )));
    }

    Ok(())
}

fn remove_sqlite_sidecars(db_file: &Path) -> std::io::Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let path = if suffix.is_empty() {
            db_file.to_path_buf()
        } else {
            PathBuf::from(format!("{}{}", db_file.display(), suffix))
        };
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn load_backup_metadata(backup_dir: &Path, filename: &str) -> Result<BackupInfo, AppError> {
    let path = metadata_path_for_backup(backup_dir, filename);
    let bytes = std::fs::read(&path)
        .map_err(|error| AppError::NotFound(format!("backup metadata not found: {error}")))?;
    serde_json::from_slice::<BackupInfo>(&bytes)
        .map_err(|error| AppError::Repository(format!("backup metadata is invalid: {error}")))
}

async fn ensure_setup_mode(app: &AppUseCase) -> Result<(), AppError> {
    if app.setup_complete().await? {
        return Err(AppError::Validation(
            "restore is only available while setup is still incomplete".into(),
        ));
    }
    Ok(())
}

fn ensure_restore_supported(_engine: DatastoreEngine) -> Result<(), AppError> {
    Ok(())
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

fn stage_restore_bundle(
    data_dir: PathBuf,
    bundle_path: PathBuf,
    datastore_config: DatastoreConfig,
    password: Option<String>,
) -> Result<RestoreSummaryResponse, AppError> {
    if datastore_config.engine == DatastoreEngine::Postgres {
        let secrets_path = managed_instance_secrets_path(&data_dir);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                AppError::Repository(format!("failed to start restore runtime: {error}"))
            })?;

        return runtime.block_on(async move {
            let prepared = restore_backup_bundle_to_datastore(
                datastore_config,
                &bundle_path,
                password.as_deref(),
            )
            .await?;

            write_owner_only_file_atomically(&secrets_path, &prepared.instance_secrets_env())?;

            Ok::<_, AppError>(restore_summary_response(prepared.summary()))
        });
    }

    let pending_dir = pending_restore_dir(&data_dir);
    let pending_db_path = pending_restore_db_path(&data_dir);
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

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            AppError::Repository(format!("failed to start restore runtime: {error}"))
        })?;

    let result = runtime.block_on(async move {
        let prepared = restore_backup_bundle_to_datastore_path(
            &pending_db_path,
            datastore_config.migration_mode,
            &bundle_path,
            password.as_deref(),
        )
        .await?;

        write_owner_only_file_atomically(&pending_secrets_path, &prepared.instance_secrets_env())?;

        let summary = restore_summary_response(prepared.summary());
        ensure_owner_only_permissions(&pending_db_path).map_err(|error| {
            AppError::Repository(format!(
                "failed to protect pending restore database: {error}"
            ))
        })?;
        Ok::<_, AppError>(summary)
    });

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

async fn write_uploaded_bundle(
    mut multipart: Multipart,
    bundle_path: &Path,
) -> Result<Option<String>, AppError> {
    let mut password = None;
    let mut bundle_written = false;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| AppError::Validation(format!("invalid multipart upload: {error}")))?
    {
        match field.name() {
            Some("password") => {
                password = normalize_password(Some(field.text().await.map_err(|error| {
                    AppError::Validation(format!("invalid password field: {error}"))
                })?));
            }
            Some("bundle") => {
                let mut output = tokio::fs::File::create(bundle_path)
                    .await
                    .map_err(|error| {
                        AppError::Repository(format!(
                            "failed to create restore staging bundle: {error}"
                        ))
                    })?;
                let mut field = field;
                while let Some(chunk) = field.chunk().await.map_err(|error| {
                    AppError::Validation(format!("failed to read restore bundle upload: {error}"))
                })? {
                    output.write_all(&chunk).await.map_err(|error| {
                        AppError::Repository(format!(
                            "failed to write restore staging bundle: {error}"
                        ))
                    })?;
                }
                output.flush().await.map_err(|error| {
                    AppError::Repository(format!("failed to flush restore staging bundle: {error}"))
                })?;
                ensure_owner_only_permissions(bundle_path).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to protect restore staging bundle: {error}"
                    ))
                })?;
                bundle_written = true;
            }
            _ => {}
        }
    }

    if !bundle_written {
        return Err(AppError::Validation(
            "restore upload is missing the backup bundle file".into(),
        ));
    }

    Ok(password)
}

pub(crate) async fn download_backup_handler(
    State(state): State<BackupRouteState>,
    headers: HeaderMap,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    AxumPath(filename): AxumPath<String>,
) -> Response {
    if let Err(error) = resolve_actor_with_app_permission(
        &state.app,
        &state.auth_runtime,
        &headers,
        Some(remote_addr),
        AppPermission::ManageSystemSettings,
    )
    .await
    {
        return map_app_error(error);
    }

    let backup_dir = state.app.backup_dir();
    if !is_supported_backup_filename(&filename) {
        return map_app_error(AppError::Validation("invalid backup filename".into()));
    }
    let info = match load_backup_metadata(&backup_dir, &filename) {
        Ok(info) => info,
        Err(error) => return map_app_error(error),
    };

    match info.status {
        BackupStatus::Creating => {
            return conflict_response("backup bundle is still being created");
        }
        BackupStatus::Failed => {
            return conflict_response(
                info.error_message
                    .unwrap_or_else(|| "backup bundle creation failed".to_string()),
            );
        }
        BackupStatus::Ready => {}
    }

    let bundle = backup_dir.join(&filename);
    let file = match tokio::fs::File::open(&bundle).await {
        Ok(file) => file,
        Err(error) => {
            return map_app_error(AppError::NotFound(format!(
                "backup bundle not found: {error}"
            )));
        }
    };
    let content_length = match file.metadata().await {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            return map_app_error(AppError::Repository(format!(
                "failed to stat backup bundle: {error}"
            )));
        }
    };

    let stream = ReaderStream::new(file);
    let mut response = Body::from_stream(stream).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if let Ok(value) = HeaderValue::from_str(&content_length.to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

pub(crate) async fn setup_restore_inspect_handler(
    State(state): State<BackupRouteState>,
    headers: HeaderMap,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    multipart: Multipart,
) -> Response {
    if let Err(error) = resolve_actor_with_app_permission(
        &state.app,
        &state.auth_runtime,
        &headers,
        Some(remote_addr),
        AppPermission::ManageSystemSettings,
    )
    .await
    {
        return map_app_error(error);
    }
    if let Err(error) = ensure_setup_mode(&state.app).await {
        return map_app_error(error);
    }
    if let Err(error) = ensure_restore_supported(state.datastore_engine) {
        return map_app_error(error);
    }

    prune_stale_restore_uploads(&state.data_dir);
    let upload_id = next_restore_upload_id();
    let upload_dir = restore_upload_dir(&state.data_dir, &upload_id);
    if let Err(error) = tokio::fs::create_dir_all(&upload_dir).await {
        return map_app_error(AppError::Repository(format!(
            "failed to create restore staging directory: {error}"
        )));
    }
    if let Err(error) = ensure_owner_only_dir_permissions(&upload_dir) {
        return map_app_error(AppError::Repository(format!(
            "failed to protect restore staging directory: {error}"
        )));
    }
    let bundle_path = upload_dir.join(STAGED_BUNDLE_FILENAME);
    let password = match write_uploaded_bundle(multipart, &bundle_path).await {
        Ok(password) => password,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&upload_dir).await;
            return map_app_error(error);
        }
    };

    let summary = match inspect_backup_bundle(&bundle_path, password.as_deref()) {
        Ok(summary) => summary,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&upload_dir).await;
            return map_app_error(error);
        }
    };

    Json(RestoreInspectResponse {
        upload_id,
        summary: restore_summary_response(&summary),
    })
    .into_response()
}

pub(crate) async fn setup_restore_apply_handler(
    State(state): State<BackupRouteState>,
    headers: HeaderMap,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    body: Bytes,
) -> Response {
    if let Err(error) = resolve_actor_with_app_permission(
        &state.app,
        &state.auth_runtime,
        &headers,
        Some(remote_addr),
        AppPermission::ManageSystemSettings,
    )
    .await
    {
        return map_app_error(error);
    }
    if let Err(error) = ensure_setup_mode(&state.app).await {
        return map_app_error(error);
    }
    if let Err(error) = ensure_restore_supported(state.datastore_engine) {
        return map_app_error(error);
    }

    let request = match serde_json::from_slice::<RestoreApplyRequest>(&body) {
        Ok(request) => request,
        Err(error) => {
            return map_app_error(AppError::Validation(format!(
                "invalid restore apply request body: {error}"
            )));
        }
    };

    let upload_id = request.upload_id.trim();
    if upload_id.is_empty() || upload_id.contains('/') || upload_id.contains('\\') {
        return map_app_error(AppError::Validation("invalid restore upload id".into()));
    }

    let bundle_path = restore_upload_bundle_path(&state.data_dir, upload_id);
    if !bundle_path.exists() {
        return map_app_error(AppError::NotFound(
            "restore upload could not be found; inspect the bundle again".into(),
        ));
    }

    let password = normalize_password(request.password);
    let data_dir = state.data_dir.clone();
    let datastore_config = state.datastore_config.clone();
    let summary = match tokio::task::spawn_blocking(move || {
        stage_restore_bundle(data_dir, bundle_path, datastore_config, password)
    })
    .await
    {
        Ok(Ok(summary)) => summary,
        Ok(Err(error)) => return map_app_error(error),
        Err(error) => {
            return map_app_error(AppError::Repository(format!(
                "restore worker failed: {error}"
            )));
        }
    };

    let _ = tokio::fs::remove_dir_all(restore_upload_dir(&state.data_dir, upload_id)).await;

    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(250)).await;
        std::process::exit(0);
    });

    (StatusCode::ACCEPTED, Json(RestoreApplyResponse { summary })).into_response()
}

pub(crate) fn finalize_pending_restore_if_present(
    data_dir: &Path,
    db_path: &str,
) -> std::io::Result<bool> {
    let pending_dir = pending_restore_dir(data_dir);
    let pending_db = pending_restore_db_path(data_dir);
    let pending_ready = pending_restore_ready_path(data_dir);
    if !pending_db.exists() {
        return Ok(false);
    }
    if !pending_ready.exists() {
        let _ = std::fs::remove_dir_all(&pending_dir);
        return Ok(false);
    }

    let pending_secrets = pending_restore_instance_secrets_path(data_dir);
    if !pending_secrets.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "pending restore secrets are missing",
        ));
    }

    let target_secrets = managed_instance_secrets_path(data_dir);
    if target_secrets.exists() {
        std::fs::remove_file(&target_secrets)?;
    }
    std::fs::copy(&pending_secrets, &target_secrets)?;
    ensure_owner_only_permissions(&target_secrets)?;

    let target_db = datastore_file_path(db_path);
    if let Some(parent) = target_db.parent() {
        std::fs::create_dir_all(parent)?;
    }
    remove_sqlite_sidecars(&target_db)?;
    std::fs::rename(&pending_db, &target_db)?;
    ensure_owner_only_permissions(&target_db)?;

    let _ = std::fs::remove_dir_all(&pending_dir);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_pending_restore_ignores_incomplete_restore_without_marker() {
        let temp = tempfile::tempdir().expect("tempdir");
        let data_dir = temp.path();
        let pending_dir = pending_restore_dir(data_dir);
        std::fs::create_dir_all(&pending_dir).expect("pending dir");
        std::fs::write(pending_restore_db_path(data_dir), b"partial").expect("pending db");

        let target_db = data_dir.join("scryer.db");
        std::fs::write(&target_db, b"original").expect("target db");

        let restored = finalize_pending_restore_if_present(
            data_dir,
            &format!("sqlite://{}", target_db.display()),
        )
        .expect("finalize");

        assert!(!restored);
        assert_eq!(std::fs::read(&target_db).expect("target db"), b"original");
        assert!(!pending_dir.exists());
    }

    #[test]
    fn finalize_pending_restore_requires_marker_and_preserves_restored_secrets() {
        let temp = tempfile::tempdir().expect("tempdir");
        let data_dir = temp.path();
        let pending_dir = pending_restore_dir(data_dir);
        std::fs::create_dir_all(&pending_dir).expect("pending dir");
        std::fs::write(pending_restore_db_path(data_dir), b"restored").expect("pending db");
        std::fs::write(
            pending_restore_instance_secrets_path(data_dir),
            b"SCRYER_ENCRYPTION_KEY=\"restored\"\n",
        )
        .expect("pending secrets");
        std::fs::write(pending_restore_ready_path(data_dir), b"ready").expect("ready marker");

        let target_db = data_dir.join("scryer.db");
        let target_secrets = managed_instance_secrets_path(data_dir);
        std::fs::write(&target_db, b"original").expect("target db");
        std::fs::write(&target_secrets, b"old").expect("target secrets");

        let restored = finalize_pending_restore_if_present(
            data_dir,
            &format!("sqlite://{}", target_db.display()),
        )
        .expect("finalize");

        assert!(restored);
        assert_eq!(std::fs::read(&target_db).expect("target db"), b"restored");
        assert_eq!(
            std::fs::read_to_string(&target_secrets).expect("target secrets"),
            "SCRYER_ENCRYPTION_KEY=\"restored\"\n",
        );
        assert!(!pending_dir.exists());
    }

    #[test]
    fn supported_backup_filenames_use_new_encrypted_extension() {
        assert!(is_supported_backup_filename(
            "scryer_backup_20260514_010203_123.scryer-backup.tar.zst"
        ));
        assert!(is_supported_backup_filename(
            "scryer_backup_20260514_010203_123.scryer-backup.enc"
        ));
        assert!(!is_supported_backup_filename("random-backup.bin"));
    }

    #[test]
    fn normalize_password_preserves_exact_bytes() {
        assert_eq!(
            normalize_password(Some("  exact password  ".to_string())),
            Some("  exact password  ".to_string())
        );
        assert_eq!(normalize_password(Some(String::new())), Some(String::new()));
    }
}
