use crate::{AppError, AppResult};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub const RECYCLE_MANIFEST_SCHEMA: &str = "scryer.recycle-entry.v1";
pub const RECYCLE_STATUS_PENDING: &str = "pending";
pub const RECYCLE_STATUS_COMMITTED: &str = "committed";
pub const RECYCLE_STATUS_QUARANTINED: &str = "quarantined";

const RECYCLE_ROOT_SENTINEL: &str = ".scryer-recycle-root";
const DEFAULT_RETENTION_DAYS: u32 = 7;
const RECYCLE_DIR_NAME: &str = ".scryer-recycle";

/// Configuration for the recycle bin, resolved from application settings.
pub struct RecycleBinConfig {
    pub enabled: bool,
    pub base_path: PathBuf,
    pub retention_days: u32,
    pub cleanup_enabled: bool,
    pub validation_error: Option<String>,
    /// Allowlist of configured media roots the source file must live under before
    /// it may be recycled or (when recycling is disabled) permanently deleted.
    /// An empty list means the roots are unknown (legacy/misconfigured) and the
    /// containment check is skipped with a warning rather than refusing.
    pub source_roots: Vec<PathBuf>,
}

/// Metadata written alongside each recycled file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecycleManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_operation_id: Option<String>,
    pub recycled_at: String,
    pub original_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_file_id: Option<String>,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_root: Option<String>,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement_file_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement_path: Option<String>,
}

/// Result of a successful recycle operation.
#[derive(Debug, Clone)]
pub struct RecycleResult {
    pub entry_id: String,
    pub entry_dir: PathBuf,
    pub recycled_path: PathBuf,
    pub manifest_path: PathBuf,
}

/// A committed recycle entry that passed local recycle-root checks.
#[derive(Debug, Clone)]
pub struct CommittedRecycleEntry {
    pub entry_dir: PathBuf,
    pub manifest: RecycleManifest,
}

/// A recycle bin entry for listing purposes.
#[derive(Debug, Clone)]
pub struct RecycleEntry {
    /// Directory name, e.g. "20260307_120015437_abc123".
    pub entry_id: String,
    pub manifest: RecycleManifest,
    /// Which media root this entry belongs to.
    pub media_root: String,
}

pub fn validate_recycle_entry_id(entry_id: &str) -> AppResult<()> {
    if entry_id.is_empty()
        || entry_id == "."
        || entry_id == ".."
        || !entry_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(AppError::Validation(
            "recycle entry id is not a valid opaque id".into(),
        ));
    }
    Ok(())
}

impl RecycleManifest {
    pub fn pending_upgrade(
        original_path: String,
        original_file_id: String,
        size_bytes: u64,
        title_id: String,
        media_root: Option<String>,
    ) -> Self {
        Self {
            schema: None,
            entry_id: None,
            source_operation_id: Some(scryer_domain::Id::new().0),
            recycled_at: Utc::now().to_rfc3339(),
            original_path,
            original_file_id: Some(original_file_id),
            size_bytes,
            title_id: Some(title_id),
            media_root,
            reason: "upgrade_replaced".to_string(),
            status: Some(RECYCLE_STATUS_PENDING.to_string()),
            replacement_file_id: None,
            replacement_path: None,
        }
    }

    fn is_schema_current(&self) -> bool {
        self.schema.as_deref() == Some(RECYCLE_MANIFEST_SCHEMA)
    }

    fn is_committed(&self) -> bool {
        self.status.as_deref() == Some(RECYCLE_STATUS_COMMITTED)
    }

    fn is_quarantined(&self) -> bool {
        self.status.as_deref() == Some(RECYCLE_STATUS_QUARANTINED)
    }
}

fn manifest_path(entry_dir: &Path) -> PathBuf {
    entry_dir.join("manifest.json")
}

fn sentinel_path(config: &RecycleBinConfig) -> PathBuf {
    config.base_path.join(RECYCLE_ROOT_SENTINEL)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(segment) => normalized.push(segment),
        }
    }
    normalized
}

fn generated_entry_id(entry_id: &str) -> bool {
    let mut parts = entry_id.split('_');
    let Some(date) = parts.next() else {
        return false;
    };
    let Some(time) = parts.next() else {
        return false;
    };
    let Some(suffix) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && date.len() == 8
        && date.bytes().all(|byte| byte.is_ascii_digit())
        && time.len() == 9
        && time.bytes().all(|byte| byte.is_ascii_digit())
        && suffix.len() == 6
        && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn cleanup_ready(config: &RecycleBinConfig) -> bool {
    config.enabled
        && config.cleanup_enabled
        && config.base_path.exists()
        && sentinel_path(config).exists()
}

async fn ensure_recycle_root(config: &RecycleBinConfig) -> AppResult<()> {
    tokio::fs::create_dir_all(&config.base_path)
        .await
        .map_err(|e| {
            AppError::Repository(format!(
                "failed to create recycle directory {}: {}",
                config.base_path.display(),
                e
            ))
        })?;

    let sentinel = sentinel_path(config);
    if !sentinel.exists() {
        tokio::fs::write(&sentinel, RECYCLE_MANIFEST_SCHEMA.as_bytes())
            .await
            .map_err(|e| {
                AppError::Repository(format!(
                    "failed to write recycle root sentinel {}: {}",
                    sentinel.display(),
                    e
                ))
            })?;
    }
    Ok(())
}

fn trusted_committed_entry(
    config: &RecycleBinConfig,
    entry_dir: &Path,
    manifest: &RecycleManifest,
) -> Result<(), String> {
    if !cleanup_ready(config) {
        return Err(config
            .validation_error
            .clone()
            .unwrap_or_else(|| "recycle root is not enabled for cleanup".to_string()));
    }

    let entry_name = entry_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "recycle entry has no valid directory name".to_string())?;
    validate_recycle_entry_id(entry_name).map_err(|error| error.to_string())?;
    if !generated_entry_id(entry_name) {
        return Err("recycle entry directory was not generated by Scryer".to_string());
    }
    if !manifest.is_schema_current() {
        return Err("recycle manifest schema is missing or unsupported".to_string());
    }
    if manifest.entry_id.as_deref() != Some(entry_name) {
        return Err("recycle manifest entry id does not match directory".to_string());
    }
    if !manifest.is_committed() {
        return Err("recycle entry is not committed".to_string());
    }

    let expected_parent = normalize_path(&config.base_path);
    let actual_parent = entry_dir
        .parent()
        .map(normalize_path)
        .ok_or_else(|| "recycle entry has no parent directory".to_string())?;
    if actual_parent != expected_parent {
        return Err("recycle entry is outside the configured recycle root".to_string());
    }

    Ok(())
}

async fn quarantine_untrusted_committed_entry(
    config: &RecycleBinConfig,
    entry_dir: &Path,
    manifest: &RecycleManifest,
    reason: &str,
) -> AppResult<bool> {
    if !cleanup_ready(config) || !manifest.is_committed() {
        return Ok(false);
    }

    let Some(parent) = entry_dir.parent() else {
        return Ok(false);
    };
    if normalize_path(parent) != normalize_path(&config.base_path) {
        return Ok(false);
    }

    warn!(
        path = %entry_dir.display(),
        reason = %reason,
        "quarantining untrusted committed recycle entry"
    );
    quarantine_entry(entry_dir, manifest, reason).await?;
    Ok(true)
}

async fn read_manifest(entry_dir: &Path) -> AppResult<Option<RecycleManifest>> {
    let path = manifest_path(entry_dir);
    if !path.exists() {
        return Ok(None);
    }
    let manifest_bytes = tokio::fs::read(&path).await.map_err(|e| {
        AppError::Repository(format!(
            "failed to read recycle manifest {}: {}",
            path.display(),
            e
        ))
    })?;
    let manifest: RecycleManifest = serde_json::from_slice(&manifest_bytes).map_err(|e| {
        AppError::Repository(format!(
            "failed to parse recycle manifest {}: {}",
            path.display(),
            e
        ))
    })?;
    Ok(Some(manifest))
}

async fn write_manifest(entry_dir: &Path, manifest: &RecycleManifest) -> AppResult<()> {
    let path = manifest_path(entry_dir);
    let manifest_json = serde_json::to_string_pretty(manifest).map_err(|e| {
        AppError::Repository(format!("failed to serialize recycle manifest: {}", e))
    })?;
    tokio::fs::write(&path, manifest_json.as_bytes())
        .await
        .map_err(|e| {
            AppError::Repository(format!(
                "failed to write recycle manifest {}: {}",
                path.display(),
                e
            ))
        })
}

/// Move a file to the recycle bin instead of deleting it.
///
/// If the recycle bin is disabled or its cleanup path is invalid, returns an error instead
/// of deleting user content directly.
///
/// If the file does not exist, returns `Ok(None)` without error (matches the current
/// `ErrorKind::NotFound` handling in callers).
pub async fn recycle_file(
    config: &RecycleBinConfig,
    source_path: &Path,
    mut manifest: RecycleManifest,
) -> AppResult<Option<RecycleResult>> {
    // If the source doesn't exist, nothing to recycle.
    if !source_path.exists() {
        return Ok(None);
    }

    // Refuse to act on a path outside the configured media roots. This guards both
    // the permanent-delete branch (recycle disabled) and the recycle move against a
    // stale/corrupt/out-of-root source path.
    ensure_source_within_roots(config, source_path)?;

    if !config.enabled {
        if let Err(err) = tokio::fs::remove_file(source_path).await
            && err.kind() != std::io::ErrorKind::NotFound
        {
            return Err(AppError::Repository(format!(
                "failed to delete file {}: {}",
                source_path.display(),
                err
            )));
        }
        return Ok(None);
    }

    if !config.cleanup_enabled {
        return Err(AppError::Validation(format!(
            "refusing to recycle {} because the recycle bin path is unsafe: {}",
            source_path.display(),
            config
                .validation_error
                .as_deref()
                .unwrap_or("invalid recycle bin configuration")
        )));
    }

    // Build timestamped directory name: YYYYMMDD_HHMMSSmmm_<6-char-id>
    let now = Utc::now();
    let full_id = scryer_domain::Id::new().0;
    let short_id = &full_id[..6];
    let dir_name = format!("{}_{}", now.format("%Y%m%d_%H%M%S%3f"), short_id);
    let recycle_dir = config.base_path.join(&dir_name);

    ensure_recycle_root(config).await?;
    tokio::fs::create_dir_all(&recycle_dir).await.map_err(|e| {
        AppError::Repository(format!(
            "failed to create recycle directory {}: {}",
            recycle_dir.display(),
            e
        ))
    })?;

    manifest.schema = Some(RECYCLE_MANIFEST_SCHEMA.to_string());
    manifest.entry_id = Some(dir_name.clone());
    manifest
        .source_operation_id
        .get_or_insert_with(|| scryer_domain::Id::new().0);
    manifest
        .status
        .get_or_insert_with(|| RECYCLE_STATUS_COMMITTED.to_string());
    write_manifest(&recycle_dir, &manifest).await?;

    // Move the file into the recycle directory
    let file_name = Path::new(&manifest.original_path)
        .file_name()
        .or_else(|| source_path.file_name())
        .unwrap_or_else(|| std::ffi::OsStr::new("unknown"));
    let recycled_path = recycle_dir.join(file_name);

    // Try rename first (instant if same filesystem)
    match tokio::fs::rename(source_path, &recycled_path).await {
        Ok(()) => {}
        Err(rename_err) => {
            // Cross-device: fall back to copy + delete
            warn!(
                error = %rename_err,
                "rename failed (likely cross-device), falling back to copy"
            );
            tokio::fs::copy(source_path, &recycled_path)
                .await
                .map_err(|e| {
                    AppError::Repository(format!(
                        "failed to copy {} to recycle bin {}: {}",
                        source_path.display(),
                        recycled_path.display(),
                        e
                    ))
                })?;
            // Prove the recycled copy is identical before deleting the original.
            // On mismatch, discard the partial recycle entry and keep the source.
            if let Err(verify_error) =
                crate::fs_integrity::verify_same_file_async(source_path, &recycled_path).await
            {
                let _ = tokio::fs::remove_dir_all(&recycle_dir).await;
                return Err(verify_error);
            }
            tokio::fs::remove_file(source_path).await.map_err(|e| {
                AppError::Repository(format!(
                    "failed to remove source file {} after copy to recycle bin: {}",
                    source_path.display(),
                    e
                ))
            })?;
        }
    }

    info!(
        original = %source_path.display(),
        recycled = %recycled_path.display(),
        reason = %manifest.reason,
        "file moved to recycle bin"
    );

    Ok(Some(RecycleResult {
        entry_id: dir_name,
        entry_dir: recycle_dir.clone(),
        recycled_path,
        manifest_path: manifest_path(&recycle_dir),
    }))
}

/// Lexically normalize a path (collapse `.`/`..`, no filesystem access), matching
/// the normalization applied to configured media roots.
fn lexically_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(segment) => normalized.push(segment),
        }
    }
    normalized
}

/// Refuse to recycle/delete a source path that is not inside any configured media
/// root. Source-removing operations must fail closed when no roots are available.
pub(crate) fn ensure_source_within_roots(
    config: &RecycleBinConfig,
    source_path: &Path,
) -> AppResult<()> {
    if config.source_roots.is_empty() {
        return Err(AppError::Validation(format!(
            "refusing to delete {} because no configured media roots are available",
            source_path.display()
        )));
    }
    let normalized_source = lexically_normalize(source_path);
    if config
        .source_roots
        .iter()
        .any(|root| normalized_source.starts_with(root))
    {
        return Ok(());
    }
    Err(AppError::Validation(format!(
        "refusing to delete {} because it is outside the configured media roots",
        source_path.display()
    )))
}

/// Pick a non-colliding restore destination by inserting `-restored` before the
/// extension (`<stem>-restored.<ext>`, then `-restored-2`, … on repeat).
fn resolve_restored_path(original_path: &Path) -> PathBuf {
    let parent = original_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let stem = original_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "restored".to_string());
    let extension = original_path
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned());

    for counter in 1..=10_000u32 {
        let suffix = if counter == 1 {
            "-restored".to_string()
        } else {
            format!("-restored-{counter}")
        };
        let file_name = match &extension {
            Some(ext) => format!("{stem}{suffix}.{ext}"),
            None => format!("{stem}{suffix}"),
        };
        let candidate = parent.join(file_name);
        if !candidate.exists() {
            return candidate;
        }
    }

    // Pathological fallback: guarantee uniqueness with a generated id.
    let unique = scryer_domain::Id::new().0;
    let file_name = match &extension {
        Some(ext) => format!("{stem}-restored-{unique}.{ext}"),
        None => format!("{stem}-restored-{unique}"),
    };
    parent.join(file_name)
}

/// Restore a file from the recycle bin.
///
/// When `overwrite` is false and a live file already occupies `original_path`,
/// the restored file is placed at a `-restored` sibling path instead of clobbering
/// the occupant. Returns the path the file was actually restored to.
pub async fn restore_from_recycle(
    recycled_path: &Path,
    original_path: &Path,
    overwrite: bool,
) -> AppResult<PathBuf> {
    if !recycled_path.exists() {
        return Err(AppError::Repository(format!(
            "recycled file not found: {}",
            recycled_path.display()
        )));
    }

    // Ensure parent directory exists
    if let Some(parent) = original_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            AppError::Repository(format!(
                "failed to create parent directory {}: {}",
                parent.display(),
                e
            ))
        })?;
    }

    // Never silently clobber a live file at the original location unless the caller
    // explicitly opts into overwrite; divert to a `-restored` sibling instead.
    let destination = if overwrite || !original_path.exists() {
        original_path.to_path_buf()
    } else {
        let diverted = resolve_restored_path(original_path);
        warn!(
            original = %original_path.display(),
            restored_to = %diverted.display(),
            "original path is occupied; restoring to a -restored sibling to avoid overwriting the live file"
        );
        diverted
    };

    match tokio::fs::rename(recycled_path, &destination).await {
        Ok(()) => {}
        Err(_) => {
            // Cross-device fallback
            tokio::fs::copy(recycled_path, &destination)
                .await
                .map_err(|e| {
                    AppError::Repository(format!(
                        "failed to restore {} to {}: {}",
                        recycled_path.display(),
                        destination.display(),
                        e
                    ))
                })?;
            // Prove the restored copy is identical before removing the recycled
            // source. On mismatch, remove the bad restore and keep the recycled
            // copy so the file is never lost.
            if let Err(verify_error) =
                crate::fs_integrity::verify_same_file_async(recycled_path, &destination).await
            {
                let _ = tokio::fs::remove_file(&destination).await;
                return Err(verify_error);
            }
            let _ = tokio::fs::remove_file(recycled_path).await;
        }
    }

    info!(
        restored = %destination.display(),
        "file restored from recycle bin"
    );

    Ok(destination)
}

pub async fn commit_recycle_entry(
    recycle_result: &Option<RecycleResult>,
    replacement_file_id: &str,
    replacement_path: &Path,
) -> AppResult<()> {
    let Some(result) = recycle_result else {
        return Ok(());
    };

    if !replacement_path.exists() {
        return Err(AppError::Repository(format!(
            "refusing to commit recycle entry {} because replacement file does not exist: {}",
            result.entry_id,
            replacement_path.display()
        )));
    }

    let mut manifest = read_manifest(&result.entry_dir).await?.ok_or_else(|| {
        AppError::Repository(format!("missing recycle manifest {}", result.entry_id))
    })?;
    manifest.status = Some(RECYCLE_STATUS_COMMITTED.to_string());
    manifest.replacement_file_id = Some(replacement_file_id.to_string());
    manifest.replacement_path = Some(replacement_path.display().to_string());
    write_manifest(&result.entry_dir, &manifest).await
}

pub async fn quarantine_entry(
    entry_dir: &Path,
    manifest: &RecycleManifest,
    reason: &str,
) -> AppResult<()> {
    if manifest.is_quarantined() {
        return Ok(());
    }

    let mut quarantined = manifest.clone();
    quarantined.status = Some(RECYCLE_STATUS_QUARANTINED.to_string());
    quarantined.reason = format!("{}; quarantine: {}", quarantined.reason, reason);
    write_manifest(entry_dir, &quarantined).await
}

pub async fn list_committed_entries(
    config: &RecycleBinConfig,
) -> AppResult<Vec<CommittedRecycleEntry>> {
    if !cleanup_ready(config) {
        if let Some(error) = &config.validation_error {
            warn!(error = %error, path = %config.base_path.display(), "recycle bin cleanup disabled");
        }
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let mut entries = tokio::fs::read_dir(&config.base_path).await.map_err(|e| {
        AppError::Repository(format!(
            "failed to read recycle bin directory {}: {}",
            config.base_path.display(),
            e
        ))
    })?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| AppError::Repository(format!("failed to read recycle bin entry: {}", e)))?
    {
        let entry_dir = entry.path();
        if !entry_dir.is_dir() {
            continue;
        }

        let Some(manifest) = (match read_manifest(&entry_dir).await {
            Ok(manifest) => manifest,
            Err(error) => {
                warn!(path = %entry_dir.display(), error = %error, "failed to inspect recycle entry, skipping");
                continue;
            }
        }) else {
            continue;
        };

        if let Err(reason) = trusted_committed_entry(config, &entry_dir, &manifest) {
            if let Err(error) =
                quarantine_untrusted_committed_entry(config, &entry_dir, &manifest, &reason).await
            {
                warn!(
                    path = %entry_dir.display(),
                    error = %error,
                    "failed to quarantine untrusted recycle entry"
                );
            }
            continue;
        }

        results.push(CommittedRecycleEntry {
            entry_dir,
            manifest,
        });
    }

    Ok(results)
}

pub async fn list_expired_committed_entries(
    config: &RecycleBinConfig,
) -> AppResult<Vec<CommittedRecycleEntry>> {
    let cutoff = Utc::now() - chrono::Duration::days(config.retention_days as i64);
    let mut results = Vec::new();

    for entry in list_committed_entries(config).await? {
        let recycled_at = match chrono::DateTime::parse_from_rfc3339(&entry.manifest.recycled_at) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(_) => continue,
        };

        if recycled_at < cutoff {
            results.push(entry);
        }
    }

    Ok(results)
}

pub async fn purge_committed_entry(
    config: &RecycleBinConfig,
    entry_dir: &Path,
    manifest: &RecycleManifest,
) -> AppResult<bool> {
    if let Err(reason) = trusted_committed_entry(config, entry_dir, manifest) {
        warn!(path = %entry_dir.display(), reason = %reason, "skipping untrusted recycle entry purge");
        if let Err(error) =
            quarantine_untrusted_committed_entry(config, entry_dir, manifest, &reason).await
        {
            warn!(
                path = %entry_dir.display(),
                error = %error,
                "failed to quarantine untrusted recycle entry"
            );
        }
        return Ok(false);
    }

    tokio::fs::remove_dir_all(entry_dir).await.map_err(|e| {
        AppError::Repository(format!(
            "failed to purge recycle entry {}: {}",
            entry_dir.display(),
            e
        ))
    })?;
    Ok(true)
}

/// Purge recycled entries older than `config.retention_days`.
///
/// Returns the count of purged entries.
pub async fn purge_expired(config: &RecycleBinConfig) -> AppResult<u32> {
    let mut purged = 0u32;
    for entry in list_expired_committed_entries(config).await? {
        if entry.manifest.reason == "upgrade_replaced" {
            warn!(
                path = %entry.entry_dir.display(),
                "skipping generic purge for upgrade recycle entry; caller must validate replacement state"
            );
            continue;
        }
        if purge_committed_entry(config, &entry.entry_dir, &entry.manifest).await? {
            purged += 1;
        }
    }

    if purged > 0 {
        info!(purged, "purged expired recycle bin entries");
    }

    Ok(purged)
}

/// Purge all recycle bin entries that belong to a specific title.
///
/// Called during title deletion to clean up orphaned entries.
/// Returns the count of purged entries.
pub async fn purge_for_title(config: &RecycleBinConfig, title_id: &str) -> AppResult<u32> {
    if !cleanup_ready(config) {
        return Ok(0);
    }

    let mut purged = 0u32;

    let mut entries = tokio::fs::read_dir(&config.base_path).await.map_err(|e| {
        AppError::Repository(format!(
            "failed to read recycle bin directory {}: {}",
            config.base_path.display(),
            e
        ))
    })?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| AppError::Repository(format!("failed to read recycle bin entry: {}", e)))?
    {
        let entry_dir = entry.path();
        if !entry_dir.is_dir() {
            continue;
        }

        let manifest = match read_manifest(&entry_dir).await {
            Ok(Some(manifest)) => manifest,
            Ok(None) => continue,
            Err(e) => {
                warn!(path = %entry_dir.display(), error = %e, "failed to read recycle manifest, skipping");
                continue;
            }
        };

        if manifest.title_id.as_deref() == Some(title_id)
            && purge_committed_entry(config, &entry_dir, &manifest).await?
        {
            purged += 1;
        }
    }

    if purged > 0 {
        info!(
            purged,
            title_id, "purged recycle bin entries for deleted title"
        );
    }

    Ok(purged)
}

/// List all entries in a recycle bin directory.
pub async fn list_entries(
    config: &RecycleBinConfig,
    media_root: &str,
) -> AppResult<Vec<RecycleEntry>> {
    if !config.enabled || !config.cleanup_enabled || !config.base_path.exists() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();

    let mut entries = tokio::fs::read_dir(&config.base_path).await.map_err(|e| {
        AppError::Repository(format!(
            "failed to read recycle bin directory {}: {}",
            config.base_path.display(),
            e
        ))
    })?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| AppError::Repository(format!("failed to read recycle bin entry: {}", e)))?
    {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest = match read_manifest(&path).await {
            Ok(Some(manifest)) => manifest,
            Err(_) => continue,
            Ok(None) => continue,
        };

        let entry_id = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        results.push(RecycleEntry {
            entry_id,
            manifest,
            media_root: media_root.to_string(),
        });
    }

    results.sort_by(|a, b| b.manifest.recycled_at.cmp(&a.manifest.recycled_at));
    Ok(results)
}

/// Look up a specific recycle bin entry by its directory name.
pub async fn find_entry(
    config: &RecycleBinConfig,
    entry_id: &str,
) -> AppResult<Option<(PathBuf, RecycleManifest)>> {
    validate_recycle_entry_id(entry_id)?;
    if !config.enabled || !config.cleanup_enabled || !config.base_path.exists() {
        return Ok(None);
    }

    let entry_dir = config.base_path.join(entry_id);
    if !entry_dir.is_dir() {
        return Ok(None);
    }

    Ok(read_manifest(&entry_dir)
        .await?
        .map(|manifest| (entry_dir, manifest)))
}

/// Purge ALL recycle bin entries regardless of age.
pub async fn purge_all(config: &RecycleBinConfig) -> AppResult<u32> {
    if !cleanup_ready(config) {
        return Ok(0);
    }

    let mut purged = 0u32;

    let mut entries = tokio::fs::read_dir(&config.base_path).await.map_err(|e| {
        AppError::Repository(format!(
            "failed to read recycle bin directory {}: {}",
            config.base_path.display(),
            e
        ))
    })?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| AppError::Repository(format!("failed to read recycle bin entry: {}", e)))?
    {
        let entry_dir = entry.path();
        if !entry_dir.is_dir() {
            continue;
        }

        let manifest = match read_manifest(&entry_dir).await {
            Ok(Some(manifest)) => manifest,
            Ok(None) => continue,
            Err(error) => {
                warn!(path = %entry_dir.display(), error = %error, "failed to read recycle manifest, skipping");
                continue;
            }
        };

        if manifest.reason == "upgrade_replaced" {
            warn!(
                path = %entry_dir.display(),
                "skipping generic empty for upgrade recycle entry; caller must validate replacement state"
            );
            continue;
        }

        if purge_committed_entry(config, &entry_dir, &manifest).await? {
            purged += 1;
        }
    }

    if purged > 0 {
        info!(purged, "emptied recycle bin");
    }

    Ok(purged)
}

/// Resolve the media root path for a title's facet.
///
/// Uses the title's owning library roots, falling back to the facet default
/// roots when legacy data points at a missing library.
pub async fn media_root_for_title(
    app: &crate::AppUseCase,
    title: &scryer_domain::Title,
) -> Option<String> {
    app.default_media_root_for_title(title)
        .await
        .map_err(|error| {
            warn!(
                error = %error,
                title_id = %title.id,
                library_id = %title.library_id,
                "failed to resolve media root for title"
            );
        })
        .ok()
}

/// Build a recycle bin config from a file path by walking up to find the media root.
///
/// For use in contexts where `AppUseCase` is not available (e.g., standalone async functions).
/// Defaults: enabled=true, retention_days=7, base_path derived from file's grandparent.
pub fn config_from_file_path(file_path: &Path) -> RecycleBinConfig {
    // Walk up to the grandparent as a rough media root estimate.
    // e.g. /data/movies/Movie (2024)/Movie.mkv → /data/movies/
    let base = file_path
        .parent() // Movie (2024)/
        .and_then(|p| p.parent()) // /data/movies/
        .unwrap_or_else(|| Path::new("/tmp"));

    RecycleBinConfig {
        enabled: true,
        base_path: base.join(RECYCLE_DIR_NAME),
        retention_days: DEFAULT_RETENTION_DAYS,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: vec![base.to_path_buf()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(dir: &Path) -> RecycleBinConfig {
        let source_root = dir.parent().unwrap_or(dir).to_path_buf();
        RecycleBinConfig {
            enabled: true,
            base_path: dir.to_path_buf(),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![source_root],
        }
    }

    fn test_manifest() -> RecycleManifest {
        RecycleManifest {
            schema: None,
            entry_id: None,
            source_operation_id: None,
            recycled_at: Utc::now().to_rfc3339(),
            original_path: "/data/movies/test.mkv".to_string(),
            original_file_id: None,
            size_bytes: 1024,
            title_id: Some("title-123".to_string()),
            media_root: None,
            reason: "title_deleted".to_string(),
            status: None,
            replacement_file_id: None,
            replacement_path: None,
        }
    }

    fn committed_manifest(
        entry_id: &str,
        recycled_at: String,
        original_path: &str,
        title_id: Option<&str>,
        reason: &str,
    ) -> RecycleManifest {
        RecycleManifest {
            schema: Some(RECYCLE_MANIFEST_SCHEMA.to_string()),
            entry_id: Some(entry_id.to_string()),
            source_operation_id: Some("operation-1".to_string()),
            recycled_at,
            original_path: original_path.to_string(),
            original_file_id: None,
            size_bytes: 100,
            title_id: title_id.map(str::to_string),
            media_root: None,
            reason: reason.to_string(),
            status: Some(RECYCLE_STATUS_COMMITTED.to_string()),
            replacement_file_id: None,
            replacement_path: None,
        }
    }

    fn pending_manifest(entry_id: &str, recycled_at: String) -> RecycleManifest {
        let mut manifest = committed_manifest(
            entry_id,
            recycled_at,
            "/data/series/Show/S01E01.mkv",
            Some("title-123"),
            "upgrade_replaced",
        );
        manifest.status = Some(RECYCLE_STATUS_PENDING.to_string());
        manifest
    }

    async fn write_test_sentinel(recycle_dir: &Path) {
        tokio::fs::write(
            recycle_dir.join(RECYCLE_ROOT_SENTINEL),
            RECYCLE_MANIFEST_SCHEMA.as_bytes(),
        )
        .await
        .unwrap();
    }

    async fn write_test_entry(
        recycle_dir: &Path,
        entry_id: &str,
        manifest: &RecycleManifest,
    ) -> PathBuf {
        let entry_dir = recycle_dir.join(entry_id);
        tokio::fs::create_dir_all(&entry_dir).await.unwrap();
        tokio::fs::write(
            entry_dir.join("manifest.json"),
            serde_json::to_string(manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(entry_dir.join("media.mkv"), b"media")
            .await
            .unwrap();
        entry_dir
    }

    async fn read_test_manifest(entry_dir: &Path) -> RecycleManifest {
        let bytes = tokio::fs::read(entry_dir.join("manifest.json"))
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn committed_untrusted_entry_is_quarantined_when_listed() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let entry_id = "20260205_120000000_bad111";
        let mut manifest = committed_manifest(
            entry_id,
            Utc::now().to_rfc3339(),
            "/data/movies/Movie/Movie.mkv",
            Some("title-123"),
            "file_deleted",
        );
        manifest.schema = None;
        let entry_dir = write_test_entry(&recycle_dir, entry_id, &manifest).await;

        let config = test_config(&recycle_dir);
        let entries = list_committed_entries(&config).await.unwrap();

        assert!(entries.is_empty());
        let quarantined = read_test_manifest(&entry_dir).await;
        assert_eq!(
            quarantined.status.as_deref(),
            Some(RECYCLE_STATUS_QUARANTINED)
        );
        assert!(quarantined.reason.contains("quarantine:"));
    }

    #[tokio::test]
    async fn pending_untrusted_entry_is_not_quarantined_or_purged() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let entry_id = "20260205_120000000_pen111";
        let mut manifest = pending_manifest(entry_id, Utc::now().to_rfc3339());
        manifest.schema = None;
        let entry_dir = write_test_entry(&recycle_dir, entry_id, &manifest).await;

        let config = test_config(&recycle_dir);
        let purged = purge_expired(&config).await.unwrap();

        assert_eq!(purged, 0);
        assert!(entry_dir.exists());
        let pending = read_test_manifest(&entry_dir).await;
        assert_eq!(pending.status.as_deref(), Some(RECYCLE_STATUS_PENDING));
    }

    #[tokio::test]
    async fn cleanup_not_ready_does_not_quarantine_committed_entries() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();

        let entry_id = "20260205_120000000_nos111";
        let mut manifest = committed_manifest(
            entry_id,
            Utc::now().to_rfc3339(),
            "/data/movies/Movie/Movie.mkv",
            Some("title-123"),
            "file_deleted",
        );
        manifest.schema = None;
        let entry_dir = write_test_entry(&recycle_dir, entry_id, &manifest).await;

        let config = test_config(&recycle_dir);
        let entries = list_committed_entries(&config).await.unwrap();

        assert!(entries.is_empty());
        let unchanged = read_test_manifest(&entry_dir).await;
        assert_eq!(unchanged.status.as_deref(), Some(RECYCLE_STATUS_COMMITTED));
    }

    #[tokio::test]
    async fn purge_committed_entry_quarantines_untrusted_entry() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let entry_id = "20260205_120000000_pur111";
        let mut manifest = committed_manifest(
            entry_id,
            Utc::now().to_rfc3339(),
            "/data/movies/Movie/Movie.mkv",
            Some("title-123"),
            "file_deleted",
        );
        manifest.entry_id = Some("different-entry".to_string());
        let entry_dir = write_test_entry(&recycle_dir, entry_id, &manifest).await;

        let config = test_config(&recycle_dir);
        let purged = purge_committed_entry(&config, &entry_dir, &manifest)
            .await
            .unwrap();

        assert!(!purged);
        assert!(entry_dir.exists());
        let quarantined = read_test_manifest(&entry_dir).await;
        assert_eq!(
            quarantined.status.as_deref(),
            Some(RECYCLE_STATUS_QUARANTINED)
        );
    }

    #[tokio::test]
    async fn test_recycle_creates_dir_and_manifest() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("test.mkv");
        tokio::fs::write(&source, b"video data").await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap();

        assert!(result.is_some());
        let r = result.unwrap();
        assert!(r.recycled_path.exists());
        assert!(r.manifest_path.exists());
        assert!(!source.exists());

        // Verify manifest is valid JSON
        let bytes = tokio::fs::read(&r.manifest_path).await.unwrap();
        let m: RecycleManifest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(m.schema.as_deref(), Some(RECYCLE_MANIFEST_SCHEMA));
        assert_eq!(m.status.as_deref(), Some(RECYCLE_STATUS_COMMITTED));
        assert_eq!(m.reason, "title_deleted");
    }

    #[tokio::test]
    async fn test_recycle_disabled_deletes_directly() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("test.mkv");
        tokio::fs::write(&source, b"video data").await.unwrap();

        let config = RecycleBinConfig {
            enabled: false,
            base_path: tmp.path().join("recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![tmp.path().to_path_buf()],
        };

        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap();

        assert!(result.is_none());
        assert!(!source.exists());
    }

    #[tokio::test]
    async fn test_recycle_refuses_source_outside_configured_roots() {
        let tmp = TempDir::new().unwrap();
        let outside = tmp.path().join("outside.mkv");
        tokio::fs::write(&outside, b"video data").await.unwrap();

        // Recycle disabled (would permanently delete), but the source is not under
        // any configured media root, so it must be refused rather than deleted.
        let config = RecycleBinConfig {
            enabled: false,
            base_path: tmp.path().join("recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![tmp.path().join("media-root")],
        };

        let result = recycle_file(&config, &outside, test_manifest()).await;
        assert!(result.is_err(), "out-of-root source must be refused");
        assert!(outside.exists(), "refused source must not be deleted");
    }

    #[tokio::test]
    async fn test_recycle_refuses_source_when_roots_are_unknown() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("source.mkv");
        tokio::fs::write(&source, b"video data").await.unwrap();

        let config = RecycleBinConfig {
            enabled: false,
            base_path: tmp.path().join("recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: Vec::new(),
        };

        let result = recycle_file(&config, &source, test_manifest()).await;
        assert!(result.is_err(), "unknown roots must fail closed");
        assert!(source.exists(), "refused source must not be deleted");
    }

    #[tokio::test]
    async fn test_recycle_nonexistent_file_returns_none() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp.path().join("recycle"));

        let result = recycle_file(&config, &tmp.path().join("nope.mkv"), test_manifest())
            .await
            .unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_restore_returns_file() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("test.mkv");
        let content = b"video data for restore test";
        tokio::fs::write(&source, content).await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap()
            .unwrap();

        assert!(!source.exists());

        let restored_to = restore_from_recycle(&result.recycled_path, &source, false)
            .await
            .unwrap();

        assert_eq!(restored_to, source);
        assert!(source.exists());
        let restored = tokio::fs::read(&source).await.unwrap();
        assert_eq!(restored, content);
    }

    #[tokio::test]
    async fn test_restore_diverts_to_restored_sibling_on_conflict() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("movie.mkv");
        let recycled_content = b"the recycled (older) file";
        tokio::fs::write(&source, recycled_content).await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap()
            .unwrap();

        // A new live file now occupies the original path.
        let live_content = b"the current live file";
        tokio::fs::write(&source, live_content).await.unwrap();

        let restored_to = restore_from_recycle(&result.recycled_path, &source, false)
            .await
            .unwrap();

        // Live file must be untouched; restored file lands at a -restored sibling.
        assert_eq!(tokio::fs::read(&source).await.unwrap(), live_content);
        assert_eq!(restored_to, tmp.path().join("movie-restored.mkv"));
        assert_eq!(
            tokio::fs::read(&restored_to).await.unwrap(),
            recycled_content
        );
    }

    #[tokio::test]
    async fn test_restore_overwrite_replaces_existing() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("movie.mkv");
        let recycled_content = b"the recycled file to force back";
        tokio::fs::write(&source, recycled_content).await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap()
            .unwrap();

        tokio::fs::write(&source, b"will be overwritten")
            .await
            .unwrap();

        let restored_to = restore_from_recycle(&result.recycled_path, &source, true)
            .await
            .unwrap();

        assert_eq!(restored_to, source);
        assert_eq!(tokio::fs::read(&source).await.unwrap(), recycled_content);
    }

    #[tokio::test]
    async fn test_purge_removes_expired_only() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        // Create an "expired" entry (recycled 30 days ago)
        let old_id = "20260205_120000000_abc123";
        let old_dir = recycle_dir.join(old_id);
        tokio::fs::create_dir_all(&old_dir).await.unwrap();
        let old_manifest = committed_manifest(
            old_id,
            (Utc::now() - chrono::Duration::days(30)).to_rfc3339(),
            "/old.mkv",
            None,
            "file_deleted",
        );
        tokio::fs::write(
            old_dir.join("manifest.json"),
            serde_json::to_string(&old_manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(old_dir.join("old.mkv"), b"old")
            .await
            .unwrap();

        // Create a "fresh" entry (recycled just now)
        let new_id = "20260307_120000000_def456";
        let new_dir = recycle_dir.join(new_id);
        tokio::fs::create_dir_all(&new_dir).await.unwrap();
        let new_manifest = committed_manifest(
            new_id,
            Utc::now().to_rfc3339(),
            "/new.mkv",
            None,
            "file_deleted",
        );
        tokio::fs::write(
            new_dir.join("manifest.json"),
            serde_json::to_string(&new_manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(new_dir.join("new.mkv"), b"new")
            .await
            .unwrap();

        let config = test_config(&recycle_dir);
        let purged = purge_expired(&config).await.unwrap();

        assert_eq!(purged, 1);
        assert!(!old_dir.exists(), "expired entry should be purged");
        assert!(new_dir.exists(), "fresh entry should survive");
    }

    #[tokio::test]
    async fn test_pending_entry_is_not_purged() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let entry_id = "20260205_120000000_pendng";
        let entry_dir = recycle_dir.join(entry_id);
        tokio::fs::create_dir_all(&entry_dir).await.unwrap();
        let manifest = pending_manifest(
            entry_id,
            (Utc::now() - chrono::Duration::days(30)).to_rfc3339(),
        );
        tokio::fs::write(
            entry_dir.join("manifest.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(entry_dir.join("S01E01.mkv"), b"old")
            .await
            .unwrap();

        let config = test_config(&recycle_dir);
        let purged = purge_expired(&config).await.unwrap();

        assert_eq!(purged, 0);
        assert!(entry_dir.exists(), "pending entry must not be purged");
    }

    #[tokio::test]
    async fn test_purge_requires_recycle_root_sentinel() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();

        let entry_id = "20260205_120000000_nosent";
        let entry_dir = recycle_dir.join(entry_id);
        tokio::fs::create_dir_all(&entry_dir).await.unwrap();
        let manifest = committed_manifest(
            entry_id,
            (Utc::now() - chrono::Duration::days(30)).to_rfc3339(),
            "/old.mkv",
            None,
            "file_deleted",
        );
        tokio::fs::write(
            entry_dir.join("manifest.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(entry_dir.join("old.mkv"), b"old")
            .await
            .unwrap();

        let config = test_config(&recycle_dir);
        let purged = purge_expired(&config).await.unwrap();

        assert_eq!(purged, 0);
        assert!(
            entry_dir.exists(),
            "entries need a root sentinel before purge"
        );
    }

    #[tokio::test]
    async fn test_empty_recycle_bin_skips_malformed_legacy_and_pending_entries() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let legacy_dir = recycle_dir.join("20260205_120000000_legacy");
        tokio::fs::create_dir_all(&legacy_dir).await.unwrap();
        let mut legacy_manifest = test_manifest();
        legacy_manifest.recycled_at = (Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        tokio::fs::write(
            legacy_dir.join("manifest.json"),
            serde_json::to_string(&legacy_manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(legacy_dir.join("legacy.mkv"), b"legacy")
            .await
            .unwrap();

        let pending_id = "20260205_120000000_pendng";
        let pending_dir = recycle_dir.join(pending_id);
        tokio::fs::create_dir_all(&pending_dir).await.unwrap();
        let pending = pending_manifest(
            pending_id,
            (Utc::now() - chrono::Duration::days(30)).to_rfc3339(),
        );
        tokio::fs::write(
            pending_dir.join("manifest.json"),
            serde_json::to_string(&pending).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(pending_dir.join("pending.mkv"), b"pending")
            .await
            .unwrap();

        let malformed_dir = recycle_dir.join("20260205_120000000_badbad");
        tokio::fs::create_dir_all(&malformed_dir).await.unwrap();
        tokio::fs::write(malformed_dir.join("manifest.json"), b"{not json")
            .await
            .unwrap();
        tokio::fs::write(malformed_dir.join("bad.mkv"), b"bad")
            .await
            .unwrap();

        let config = test_config(&recycle_dir);
        let purged = purge_all(&config).await.unwrap();

        assert_eq!(purged, 0);
        assert!(legacy_dir.exists(), "legacy entry should be skipped");
        assert!(pending_dir.exists(), "pending entry should be skipped");
        assert!(malformed_dir.exists(), "malformed entry should be skipped");
    }

    #[tokio::test]
    async fn test_purge_for_title_removes_matching_only() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let match_id = "20260307_120000000_aaa111";
        let match_dir = recycle_dir.join(match_id);
        tokio::fs::create_dir_all(&match_dir).await.unwrap();
        let match_manifest = committed_manifest(
            match_id,
            Utc::now().to_rfc3339(),
            "/data/movies/Movie/Movie.mkv",
            Some("title-123"),
            "file_deleted",
        );
        tokio::fs::write(
            match_dir.join("manifest.json"),
            serde_json::to_string(&match_manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(match_dir.join("Movie.mkv"), b"data")
            .await
            .unwrap();

        let other_id = "20260307_120000000_bbb222";
        let other_dir = recycle_dir.join(other_id);
        tokio::fs::create_dir_all(&other_dir).await.unwrap();
        let other_manifest = committed_manifest(
            other_id,
            Utc::now().to_rfc3339(),
            "/data/movies/Other/Other.mkv",
            Some("title-456"),
            "file_deleted",
        );
        tokio::fs::write(
            other_dir.join("manifest.json"),
            serde_json::to_string(&other_manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(other_dir.join("Other.mkv"), b"other")
            .await
            .unwrap();

        let config = test_config(&recycle_dir);
        let purged = purge_for_title(&config, "title-123").await.unwrap();

        assert_eq!(purged, 1);
        assert!(!match_dir.exists(), "matching title entry should be purged");
        assert!(other_dir.exists(), "different title entry should survive");
    }
}
