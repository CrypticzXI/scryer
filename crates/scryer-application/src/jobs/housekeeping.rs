use super::*;
use crate::domain_events::{
    DomainEventActor, deleted_media_update, new_title_domain_event, title_context_snapshot,
};
use crate::events::retention::{
    OPERATIONAL_DOMAIN_EVENT_RETENTION_DAYS, operational_domain_event_types,
    user_facing_domain_event_types,
};
use std::collections::HashSet;
use std::path::Path;
use tracing::{info, warn};

const RELEASE_DECISION_RETENTION_DAYS: i64 = 30;
const RELEASE_ATTEMPT_RETENTION_DAYS: i64 = 90;
const DOWNLOAD_DELETE_RETENTION_DAYS: i64 = 7;

#[derive(Clone, Debug)]
struct RecycleEntryLibrary {
    id: String,
    name: String,
}

#[derive(Clone, Debug)]
struct RecycleRootLibrary {
    media_root: String,
    normalized_media_root: String,
    library: RecycleEntryLibrary,
}

fn recycle_path_is_under_root(path: &str, root: &str) -> bool {
    crate::catalog_workflow::library_path_is_under_root(path, root)
}

fn recycled_item_from_entry(
    entry: crate::recycle_bin::RecycleEntry,
    library: &RecycleEntryLibrary,
) -> RecycledItem {
    let file_name = Path::new(&entry.manifest.original_path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();

    RecycledItem {
        id: entry.entry_id,
        original_path: entry.manifest.original_path,
        file_name,
        size_bytes: entry.manifest.size_bytes,
        title_id: entry.manifest.title_id,
        reason: entry.manifest.reason,
        recycled_at: entry.manifest.recycled_at,
        media_root: entry.media_root,
        library_id: library.id.clone(),
        library_name: library.name.clone(),
    }
}

impl AppUseCase {
    /// Resolve media root paths and their recycle configs.
    async fn resolve_all_recycle_configs(
        &self,
    ) -> Vec<(String, crate::recycle_bin::RecycleBinConfig)> {
        let mut media_roots = Vec::new();
        let mut seen_roots = HashSet::new();

        match self.recycle_root_libraries().await {
            Ok(roots) => {
                for root in roots {
                    if seen_roots.insert(root.normalized_media_root) {
                        media_roots.push(root.media_root);
                    }
                }
            }
            Err(error) => {
                warn!(
                    error = %error,
                    "failed to resolve library roots for recycle bin housekeeping"
                );
            }
        }

        self.recycle_bin_configs_for_media_roots(media_roots).await
    }

    async fn recycle_root_libraries(&self) -> AppResult<Vec<RecycleRootLibrary>> {
        Ok(self
            .all_library_root_folders()
            .await?
            .into_iter()
            .map(|root| RecycleRootLibrary {
                media_root: root.path,
                normalized_media_root: root.normalized_path,
                library: RecycleEntryLibrary {
                    id: root.library_id,
                    name: root.library_name,
                },
            })
            .collect())
    }

    async fn resolve_recycle_entry_library(
        &self,
        entry: &crate::recycle_bin::RecycleEntry,
        roots: &[RecycleRootLibrary],
    ) -> AppResult<Option<RecycleEntryLibrary>> {
        if let Some(title_id) = entry.manifest.title_id.as_deref()
            && let Some(title) = self.services.catalog.titles.get_by_id(title_id).await?
            && let Some(root) = roots
                .iter()
                .find(|root| root.library.id == title.library_id)
        {
            return Ok(Some(root.library.clone()));
        }

        if let Some(root) = roots.iter().find(|root| {
            recycle_path_is_under_root(
                &entry.manifest.original_path,
                root.normalized_media_root.as_str(),
            )
        }) {
            return Ok(Some(root.library.clone()));
        }

        let normalized_media_root =
            crate::catalog_workflow::normalize_library_root_path(&entry.media_root);
        if normalized_media_root.is_empty() {
            return Ok(None);
        }

        Ok(roots
            .iter()
            .find(|root| root.normalized_media_root == normalized_media_root)
            .map(|root| root.library.clone()))
    }

    async fn require_recycle_bin_page_access(&self, actor: &User) -> AppResult<()> {
        if self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?
            || self
                .has_any_granted_library_permission(
                    actor,
                    scryer_domain::LibraryPermission::ManageTitles,
                )
                .await?
        {
            return Ok(());
        }

        Err(AppError::Unauthorized(
            "You do not have permission to view the recycle bin".to_string(),
        ))
    }

    async fn selected_recycle_library_ids(
        &self,
        actor: &User,
        library_ids: Option<Vec<String>>,
    ) -> AppResult<HashSet<String>> {
        let allowed = self
            .granted_library_ids_for_permission(
                actor,
                None,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?
            .into_iter()
            .collect::<HashSet<_>>();

        let Some(library_ids) = library_ids else {
            return Ok(allowed);
        };

        let requested = library_ids
            .into_iter()
            .map(|library_id| library_id.trim().to_string())
            .filter(|library_id| !library_id.is_empty())
            .collect::<HashSet<_>>();

        if requested.is_empty() {
            return Ok(allowed);
        }

        Ok(allowed
            .into_iter()
            .filter(|library_id| requested.contains(library_id))
            .collect())
    }

    async fn purge_expired_recycle_entries(
        &self,
        media_root: &str,
        config: &crate::recycle_bin::RecycleBinConfig,
    ) -> AppResult<u32> {
        let mut purged = 0u32;
        for entry in crate::recycle_bin::list_expired_committed_entries(config).await? {
            if self
                .purge_recycle_entry_after_validation(
                    media_root,
                    config,
                    &entry.entry_dir,
                    &entry.manifest,
                    DomainEventActor::system(),
                )
                .await?
            {
                purged += 1;
            }
        }
        Ok(purged)
    }

    async fn purge_recycle_entry_after_validation(
        &self,
        media_root: &str,
        config: &crate::recycle_bin::RecycleBinConfig,
        entry_dir: &std::path::Path,
        manifest: &crate::recycle_bin::RecycleManifest,
        actor: impl Into<DomainEventActor>,
    ) -> AppResult<bool> {
        let actor = actor.into();
        if let Err(reason) = self
            .validate_recycle_entry_before_permanent_delete(manifest)
            .await
        {
            warn!(
                media_root = %media_root,
                path = %entry_dir.display(),
                reason = %reason,
                "quarantining recycle entry that failed purge validation"
            );
            if let Err(error) =
                crate::recycle_bin::quarantine_entry(entry_dir, manifest, &reason).await
            {
                warn!(
                    media_root = %media_root,
                    path = %entry_dir.display(),
                    error = %error,
                    "failed to quarantine unsafe recycle entry"
                );
            }
            return Ok(false);
        }

        let purged = crate::recycle_bin::purge_committed_entry(config, entry_dir, manifest).await?;
        if purged {
            self.record_recycle_entry_purged_event(actor, manifest)
                .await;
        }
        Ok(purged)
    }

    async fn record_recycle_entry_purged_event(
        &self,
        actor: DomainEventActor,
        manifest: &crate::recycle_bin::RecycleManifest,
    ) {
        let Some(title_id) = manifest.title_id.as_deref() else {
            return;
        };
        let title = match self.services.catalog.titles.get_by_id(title_id).await {
            Ok(Some(title)) => title,
            Ok(None) => return,
            Err(error) => {
                warn!(
                    title_id = %title_id,
                    error = %error,
                    "recycle entry purged but title could not be loaded for audit event"
                );
                return;
            }
        };
        let event = new_title_domain_event(
            actor,
            &title,
            scryer_domain::DomainEventPayload::MediaFileDeleted(
                scryer_domain::MediaFileDeletedEventData {
                    title: title_context_snapshot(&title),
                    media_updates: vec![deleted_media_update(manifest.original_path.clone())],
                    file_id: manifest.original_file_id.clone(),
                    reason: scryer_domain::MediaFileDeletedReason::RecycleBinPurged,
                    episode_ids: Vec::new(),
                },
            ),
        );
        if let Err(error) = self.append_domain_event(event).await {
            warn!(
                title_id = %title_id,
                error = %error,
                "recycle entry purged but audit event could not be recorded"
            );
        }
    }

    async fn validate_recycle_entry_before_permanent_delete(
        &self,
        manifest: &crate::recycle_bin::RecycleManifest,
    ) -> Result<(), String> {
        if manifest.reason != "upgrade_replaced" {
            return Ok(());
        }

        let title_id = manifest
            .title_id
            .as_deref()
            .ok_or_else(|| "missing title id".to_string())?;
        let original_file_id = manifest
            .original_file_id
            .as_deref()
            .ok_or_else(|| "missing original media file id".to_string())?;
        let media_root = manifest
            .media_root
            .as_deref()
            .ok_or_else(|| "missing media root".to_string())?;
        if media_root.trim().is_empty() {
            return Err("missing media root".to_string());
        }
        if !std::path::Path::new(&manifest.original_path).starts_with(media_root) {
            return Err(format!(
                "original path is outside manifest media root: original={} root={}",
                manifest.original_path, media_root
            ));
        }

        let replacement_file_id = manifest
            .replacement_file_id
            .as_deref()
            .ok_or_else(|| "missing replacement media file id".to_string())?;
        let replacement_path = manifest
            .replacement_path
            .as_deref()
            .ok_or_else(|| "missing replacement media file path".to_string())?;
        let replacement = self
            .services
            .library
            .media_files
            .get_media_file_by_id(replacement_file_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "replacement media file row is missing".to_string())?;

        if replacement.file_path != replacement_path {
            return Err(format!(
                "replacement media file path mismatch: manifest={} db={}",
                replacement_path, replacement.file_path
            ));
        }
        if !std::path::Path::new(&replacement.file_path).exists() {
            return Err(format!(
                "replacement media file does not exist on disk: {}",
                replacement.file_path
            ));
        }
        if replacement.title_id != title_id {
            return Err(format!(
                "replacement title mismatch: manifest={} db={}",
                title_id, replacement.title_id
            ));
        }
        if !std::path::Path::new(&replacement.file_path).starts_with(media_root) {
            return Err(format!(
                "replacement path is outside manifest media root: replacement={} root={}",
                replacement.file_path, media_root
            ));
        }
        if self
            .services
            .library
            .media_files
            .get_media_file_by_id(original_file_id)
            .await
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err("original media file row is still active".to_string());
        }
        if let Some(active_at_original_path) = self
            .services
            .library
            .media_files
            .get_media_file_by_path(&manifest.original_path)
            .await
            .map_err(|error| error.to_string())?
            && active_at_original_path.id != replacement_file_id
        {
            return Err(format!(
                "original path is active for a different media file: {}",
                active_at_original_path.id
            ));
        }

        Ok(())
    }

    pub async fn run_housekeeping(&self, actor: &User) -> AppResult<HousekeepingReport> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.run_scheduled_housekeeping().await
    }

    pub(crate) async fn run_scheduled_housekeeping(&self) -> AppResult<HousekeepingReport> {
        info!("starting housekeeping");
        let general_settings = self.general_settings().await?;
        let history_retention_days = general_settings.history_retention_days as i64;
        let user_facing_domain_event_types = user_facing_domain_event_types();
        let operational_domain_event_types = operational_domain_event_types();

        // 1. Orphaned media files (file_path no longer exists on disk)
        let all_files = self
            .services
            .workflow
            .housekeeping
            .list_all_media_file_paths()
            .await?;
        let orphan_ids: Vec<String> = all_files
            .into_iter()
            .filter(|(_, path)| !std::path::Path::new(path).exists())
            .map(|(id, _)| id)
            .collect();
        let orphaned_media_files = if !orphan_ids.is_empty() {
            self.services
                .workflow
                .housekeeping
                .delete_media_files_by_ids(&orphan_ids)
                .await?
        } else {
            0
        };

        let stale_release_decisions = self
            .services
            .workflow
            .housekeeping
            .delete_release_decisions_older_than(RELEASE_DECISION_RETENTION_DAYS)
            .await?;
        let stale_release_attempts = self
            .services
            .workflow
            .housekeeping
            .delete_release_attempts_older_than(RELEASE_ATTEMPT_RETENTION_DAYS)
            .await?;

        let (
            stale_history_events,
            stale_domain_events,
            stale_download_import_artifacts,
            stale_import_history,
            stale_download_queue_deletes,
            stale_rule_set_history,
        ) = if general_settings.keep_history_forever {
            (0, 0, 0, 0, 0, 0)
        } else {
            (
                self.services
                    .workflow
                    .housekeeping
                    .delete_history_events_older_than(history_retention_days)
                    .await?,
                self.services
                    .workflow
                    .housekeeping
                    .delete_domain_events_older_than_for_types(
                        history_retention_days,
                        &user_facing_domain_event_types,
                    )
                    .await?,
                self.services
                    .workflow
                    .housekeeping
                    .delete_download_import_artifacts_older_than(history_retention_days)
                    .await?,
                self.services
                    .workflow
                    .housekeeping
                    .delete_terminal_imports_older_than(history_retention_days)
                    .await?,
                self.services
                    .workflow
                    .housekeeping
                    .delete_terminal_download_queue_commands_older_than(
                        DOWNLOAD_DELETE_RETENTION_DAYS,
                    )
                    .await?,
                self.services
                    .workflow
                    .housekeeping
                    .delete_rule_set_history_older_than(history_retention_days)
                    .await?,
            )
        };
        let stale_operational_domain_events = self
            .services
            .workflow
            .housekeeping
            .delete_domain_events_older_than_for_types(
                OPERATIONAL_DOMAIN_EVENT_RETENTION_DAYS,
                &operational_domain_event_types,
            )
            .await?;

        let stale_history_records = stale_release_decisions
            + stale_release_attempts
            + stale_operational_domain_events
            + stale_history_events
            + stale_domain_events
            + stale_download_import_artifacts
            + stale_import_history
            + stale_download_queue_deletes
            + stale_rule_set_history;

        // 3. Expired event outboxes (dispatched > 7 days ago)
        let expired_event_outboxes = self
            .services
            .workflow
            .housekeeping
            .delete_dispatched_event_outboxes_older_than(7)
            .await?;

        // 4. Stale staged NZB artifacts (> 1 hour old)
        let staged_nzb_artifacts_pruned = self
            .services
            .workflow
            .staged_nzb_store
            .prune_staged_nzbs_older_than(chrono::Utc::now() - chrono::Duration::hours(1))
            .await?;

        // 5. Purge expired recycle bin entries (per media root)
        let mut recycled_purged = 0u32;
        for (media_root, config) in self.resolve_all_recycle_configs().await {
            match self
                .purge_expired_recycle_entries(&media_root, &config)
                .await
            {
                Ok(n) => recycled_purged += n,
                Err(e) => info!(error = %e, media_root = %media_root, "recycle bin purge failed"),
            }
        }

        self.services
            .workflow
            .housekeeping
            .run_database_maintenance()
            .await?;

        let report = HousekeepingReport {
            orphaned_media_files,
            stale_release_decisions,
            stale_release_attempts,
            expired_event_outboxes,
            stale_history_events,
            stale_history_records,
            staged_nzb_artifacts_pruned,
            recycled_purged,
            ran_at: chrono::Utc::now().to_rfc3339(),
        };

        info!(
            orphaned_media_files,
            stale_release_decisions,
            stale_release_attempts,
            expired_event_outboxes,
            stale_history_events,
            stale_operational_domain_events,
            stale_domain_events,
            stale_download_import_artifacts,
            stale_import_history,
            stale_download_queue_deletes,
            stale_rule_set_history,
            stale_history_records,
            staged_nzb_artifacts_pruned,
            recycled_purged,
            "housekeeping completed"
        );

        Ok(report)
    }

    /// List all items across all recycle bins, sorted newest first.
    pub async fn list_recycled_items(
        &self,
        actor: &scryer_domain::User,
        library_ids: Option<Vec<String>>,
    ) -> AppResult<Vec<RecycledItem>> {
        self.require_recycle_bin_page_access(actor).await?;
        let selected_library_ids = self
            .selected_recycle_library_ids(actor, library_ids)
            .await?;
        if selected_library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let roots = self.recycle_root_libraries().await?;

        let mut all_entries = Vec::new();
        for (media_root, config) in self.resolve_all_recycle_configs().await {
            match crate::recycle_bin::list_entries(&config, &media_root).await {
                Ok(entries) => {
                    for entry in entries {
                        let Some(library) =
                            self.resolve_recycle_entry_library(&entry, &roots).await?
                        else {
                            continue;
                        };
                        if selected_library_ids.contains(&library.id) {
                            all_entries.push(recycled_item_from_entry(entry, &library));
                        }
                    }
                }
                Err(e) => {
                    info!(error = %e, media_root = %media_root, "failed to list recycle entries")
                }
            }
        }

        all_entries.sort_by(|a, b| b.recycled_at.cmp(&a.recycled_at));
        Ok(all_entries)
    }

    /// Restore a single recycled item back to its original path.
    pub async fn restore_recycled_item(
        &self,
        actor: &scryer_domain::User,
        entry_id: &str,
    ) -> AppResult<bool> {
        let roots = self.recycle_root_libraries().await?;

        for (media_root, config) in self.resolve_all_recycle_configs().await {
            if let Some((entry_dir, manifest)) =
                crate::recycle_bin::find_entry(&config, entry_id).await?
            {
                let entry = crate::recycle_bin::RecycleEntry {
                    entry_id: entry_id.to_string(),
                    manifest: manifest.clone(),
                    media_root,
                };
                let library = self
                    .resolve_recycle_entry_library(&entry, &roots)
                    .await?
                    .ok_or_else(|| {
                        AppError::Unauthorized("You do not have access to this library".to_string())
                    })?;
                self.require_granted_library_permission(
                    actor,
                    &library.id,
                    scryer_domain::LibraryPermission::ManageTitles,
                )
                .await?;

                let original_path = std::path::Path::new(&manifest.original_path);
                let file_name = original_path
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("unknown"));
                let recycled_file = entry_dir.join(file_name);

                if !recycled_file.exists() {
                    return Err(AppError::Repository(format!(
                        "recycled file not found in entry: {}",
                        recycled_file.display()
                    )));
                }

                // User-facing restore must never overwrite a live file at the
                // original path; restore_from_recycle diverts to a `-restored`
                // sibling on conflict and returns where it actually landed.
                let restored_to =
                    crate::recycle_bin::restore_from_recycle(&recycled_file, original_path, false)
                        .await?;
                if let Err(error) = tokio::fs::remove_dir_all(&entry_dir).await {
                    tracing::warn!(
                        error = %error,
                        entry_dir = %entry_dir.display(),
                        restored_to = %restored_to.display(),
                        "failed to remove recycle entry directory after restore"
                    );
                }
                if let Some(title_id) = manifest.title_id.as_deref() {
                    let restored_library_file = crate::LibraryFile {
                        path: restored_to.to_string_lossy().to_string(),
                        display_name: original_path
                            .file_stem()
                            .and_then(|value| value.to_str())
                            .unwrap_or_default()
                            .to_string(),
                        nfo_path: None,
                        size_bytes: tokio::fs::metadata(&restored_to)
                            .await
                            .ok()
                            .and_then(|metadata| i64::try_from(metadata.len()).ok()),
                        source_signature_scheme: None,
                        source_signature_value: None,
                    };
                    match self.services.catalog.titles.get_by_id(title_id).await {
                        Ok(Some(title)) => {
                            if let Err(error) = self
                                .scan_title_library_with_discovered_files(
                                    actor,
                                    title,
                                    vec![restored_library_file],
                                )
                                .await
                            {
                                tracing::warn!(
                                    error = %error,
                                    title_id,
                                    restored_to = %restored_to.display(),
                                    "failed to scan title after restoring recycled file"
                                );
                            }
                        }
                        Ok(None) => {
                            tracing::warn!(
                                title_id,
                                restored_to = %restored_to.display(),
                                "skipping restored file scan because the title no longer exists"
                            );
                        }
                        Err(error) => {
                            tracing::warn!(
                                error = %error,
                                title_id,
                                restored_to = %restored_to.display(),
                                "failed to load title before restored file scan"
                            );
                        }
                    }
                }
                return Ok(true);
            }
        }

        Err(AppError::NotFound(format!("recycle entry {}", entry_id)))
    }

    /// Permanently delete a single recycled item.
    pub async fn delete_recycled_item(
        &self,
        actor: &scryer_domain::User,
        entry_id: &str,
    ) -> AppResult<bool> {
        let roots = self.recycle_root_libraries().await?;

        for (media_root, config) in self.resolve_all_recycle_configs().await {
            if let Some((entry_dir, manifest)) =
                crate::recycle_bin::find_entry(&config, entry_id).await?
            {
                let entry = crate::recycle_bin::RecycleEntry {
                    entry_id: entry_id.to_string(),
                    manifest: manifest.clone(),
                    media_root: media_root.clone(),
                };
                let library = self
                    .resolve_recycle_entry_library(&entry, &roots)
                    .await?
                    .ok_or_else(|| {
                        AppError::Unauthorized("You do not have access to this library".to_string())
                    })?;
                self.require_granted_library_permission(
                    actor,
                    &library.id,
                    scryer_domain::LibraryPermission::ManageTitles,
                )
                .await?;

                return self
                    .purge_recycle_entry_after_validation(
                        &media_root,
                        &config,
                        &entry_dir,
                        &manifest,
                        actor,
                    )
                    .await;
            }
        }

        Err(AppError::NotFound(format!("recycle entry {}", entry_id)))
    }

    /// Empty all recycle bins across all media roots.
    pub async fn empty_recycle_bin(
        &self,
        actor: &scryer_domain::User,
        library_ids: Option<Vec<String>>,
    ) -> AppResult<u32> {
        self.require_recycle_bin_page_access(actor).await?;
        let selected_library_ids = self
            .selected_recycle_library_ids(actor, library_ids)
            .await?;
        if selected_library_ids.is_empty() {
            return Ok(0);
        }

        let roots = self.recycle_root_libraries().await?;

        let mut total = 0u32;
        for (media_root, config) in self.resolve_all_recycle_configs().await {
            match crate::recycle_bin::list_committed_entries(&config).await {
                Ok(entries) => {
                    for entry in entries {
                        let entry_id = entry
                            .entry_dir
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let recycle_entry = crate::recycle_bin::RecycleEntry {
                            entry_id,
                            manifest: entry.manifest.clone(),
                            media_root: media_root.clone(),
                        };
                        let Some(library) = self
                            .resolve_recycle_entry_library(&recycle_entry, &roots)
                            .await?
                        else {
                            continue;
                        };
                        if !selected_library_ids.contains(&library.id) {
                            continue;
                        }
                        match self
                            .purge_recycle_entry_after_validation(
                                &media_root,
                                &config,
                                &entry.entry_dir,
                                &entry.manifest,
                                actor,
                            )
                            .await
                        {
                            Ok(true) => total += 1,
                            Ok(false) => {}
                            Err(error) => warn!(
                                path = %entry.entry_dir.display(),
                                error = %error,
                                "failed to empty recycle entry"
                            ),
                        }
                    }
                }
                Err(e) => {
                    info!(error = %e, media_root = %media_root, "failed to empty recycle bin")
                }
            }
        }
        Ok(total)
    }
}
