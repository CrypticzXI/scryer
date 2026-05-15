use std::path::{Path, PathBuf};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path as AxumPath, RawQuery, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use scryer_application::{AppError, AppUseCase, BackupInfo, BackupService, BackupStatus};
use scryer_infrastructure::datastore_file_path;
use serde::Serialize;
use tokio_util::io::ReaderStream;

use crate::middleware::map_app_error;

const BACKUP_METADATA_EXTENSION: &str = ".metadata.json";
const BACKUP_PLAINTEXT_EXTENSION: &str = ".tar.zst";
const BACKUP_ENCRYPTED_EXTENSION: &str = ".enc";
const LEGACY_BACKUP_PLAINTEXT_EXTENSION: &str = ".scryer-backup.tar.zst";
const LEGACY_BACKUP_ENCRYPTED_EXTENSION: &str = ".scryer-backup.enc";
const INSTANCE_SECRETS_ENV_FILENAME: &str = "instance-secrets.env";
const PENDING_RESTORE_DB_FILENAME: &str = "restored-scryer.db";
const PENDING_RESTORE_DIRNAME: &str = "restore-pending";
const PENDING_RESTORE_READY_FILENAME: &str = "restore-ready";

#[derive(Clone)]
pub(crate) struct BackupRouteState {
    pub(crate) app: AppUseCase,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
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
    !filename.contains('/')
        && !filename.contains('\\')
        && (filename.ends_with(BACKUP_PLAINTEXT_EXTENSION)
            || filename.ends_with(BACKUP_ENCRYPTED_EXTENSION)
            || filename.ends_with(LEGACY_BACKUP_PLAINTEXT_EXTENSION)
            || filename.ends_with(LEGACY_BACKUP_ENCRYPTED_EXTENSION))
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

fn decode_query_component(component: &str) -> Result<String, AppError> {
    fn from_hex(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    let bytes = component.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' => {
                if index + 2 >= bytes.len() {
                    return Err(AppError::Unauthorized(
                        "backup download ticket query is invalid".into(),
                    ));
                }
                let Some(high) = from_hex(bytes[index + 1]) else {
                    return Err(AppError::Unauthorized(
                        "backup download ticket query is invalid".into(),
                    ));
                };
                let Some(low) = from_hex(bytes[index + 2]) else {
                    return Err(AppError::Unauthorized(
                        "backup download ticket query is invalid".into(),
                    ));
                };
                decoded.push((high << 4) | low);
                index += 3;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8(decoded)
        .map_err(|_| AppError::Unauthorized("backup download ticket query is invalid".into()))
}

fn parse_backup_download_ticket(raw_query: Option<&str>) -> Result<String, AppError> {
    let Some(raw_query) = raw_query else {
        return Err(AppError::Unauthorized(
            "backup download ticket is required".into(),
        ));
    };
    if raw_query.trim().is_empty() {
        return Err(AppError::Unauthorized(
            "backup download ticket is required".into(),
        ));
    }

    let mut ticket: Option<String> = None;
    for pair in raw_query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = decode_query_component(raw_key)?;
        if key != "ticket" {
            continue;
        }
        let value = decode_query_component(raw_value)?;
        if value.trim().is_empty() {
            return Err(AppError::Unauthorized(
                "backup download ticket is required".into(),
            ));
        }
        if ticket.replace(value).is_some() {
            return Err(AppError::Unauthorized(
                "backup download ticket query is invalid".into(),
            ));
        }
    }

    ticket.ok_or_else(|| AppError::Unauthorized("backup download ticket is required".into()))
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

pub(crate) async fn download_backup_handler(
    State(state): State<BackupRouteState>,
    AxumPath(filename): AxumPath<String>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let ticket = match parse_backup_download_ticket(raw_query.as_deref()) {
        Ok(ticket) => ticket,
        Err(error) => return map_app_error(error),
    };

    if let Err(error) = state
        .app
        .authorize_backup_download_ticket(&filename, &ticket)
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
        BackupStatus::Invalid => {
            return conflict_response(
                info.error_message
                    .unwrap_or_else(|| "backup bundle is invalid".to_string()),
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
    fn parse_backup_download_ticket_requires_valid_ticket_query() {
        let missing = parse_backup_download_ticket(None).unwrap_err();
        assert!(matches!(missing, AppError::Unauthorized(_)));

        let blank = parse_backup_download_ticket(Some("ticket=")).unwrap_err();
        assert!(matches!(blank, AppError::Unauthorized(_)));

        let malformed = parse_backup_download_ticket(Some("ticket=%zz")).unwrap_err();
        assert!(matches!(malformed, AppError::Unauthorized(_)));
    }

    #[test]
    fn parse_backup_download_ticket_accepts_ticket_and_ignores_other_params() {
        let ticket = parse_backup_download_ticket(Some("foo=bar&ticket=abc.def&baz=qux"))
            .expect("ticket should parse");
        assert_eq!(ticket, "abc.def");
    }

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
            "20260514_010203_123_47f908fa.tar.zst"
        ));
        assert!(is_supported_backup_filename(
            "20260514_010203_123_47f908fa.enc"
        ));
        assert!(is_supported_backup_filename(
            "scryer_backup_20260514_010203_123.scryer-backup.enc"
        ));
        assert!(!is_supported_backup_filename("random-backup.bin"));
    }
}
