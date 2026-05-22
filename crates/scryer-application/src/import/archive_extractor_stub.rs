use std::path::{Path, PathBuf};

use crate::{AppError, AppResult};

pub async fn extract_archives_if_needed(
    _dir: &Path,
    _password: Option<&str>,
) -> AppResult<Option<PathBuf>> {
    Ok(None)
}

pub fn is_password_required_error(_error: &AppError) -> bool {
    false
}

pub async fn cleanup_extracted_dir(_dir: &Path) {}
