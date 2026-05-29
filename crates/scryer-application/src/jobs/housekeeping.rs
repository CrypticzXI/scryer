use super::*;
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

fn normalize_recycle_media_root(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    #[cfg(windows)]
    {
        trimmed
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    }

    #[cfg(not(windows))]
    {
        trimmed.replace('\\', "/").trim_end_matches('/').to_string()
    }
}

fn recycle_path_is_under_root(path: &str, root: &str) -> bool {
    let normalized_path = normalize_recycle_media_root(path);
    let normalized_root = normalize_recycle_media_root(root);
    if normalized_path.is_empty() || normalized_root.is_empty() {
        return false;
    }

    #[cfg(windows)]
    let separator = "\\";
    #[cfg(not(windows))]
    let separator = "/";

    normalized_path == normalized_root
        || normalized_path.starts_with(&format!("{normalized_root}{separator}"))
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

        if !media_roots.is_empty() {
            return self.recycle_bin_configs_for_media_roots(media_roots).await;
        }

        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            let root_folders = match self.root_folders_for_facet(&facet).await {
                Ok(root_folders) => root_folders,
                Err(error) => {
                    warn!(
                        facet = facet.as_str(),
                        error = %error,
                        "failed to resolve canonical roots for recycle bin housekeeping"
                    );
                    continue;
                }
            };

            for root in root_folders {
                let media_root = root.path.trim();
                if media_root.is_empty() {
                    continue;
                }
                let normalized_media_root = normalize_recycle_media_root(media_root);
                if !seen_roots.insert(normalized_media_root) {
                    continue;
                }
                media_roots.push(media_root.to_string());
            }
        }
        self.recycle_bin_configs_for_media_roots(media_roots).await
    }

    async fn recycle_root_libraries(&self) -> AppResult<Vec<RecycleRootLibrary>> {
        let libraries = self.services.catalog.libraries.list(None).await?;
        let mut roots = Vec::new();

        for library in libraries {
            let library_context = RecycleEntryLibrary {
                id: library.id,
                name: library.name,
            };

            for root in library.roots {
                let media_root = root.path.trim().to_string();
                let normalized_media_root = normalize_recycle_media_root(&media_root);
                if normalized_media_root.is_empty() {
                    continue;
                }
                roots.push(RecycleRootLibrary {
                    media_root,
                    normalized_media_root,
                    library: library_context.clone(),
                });
            }
        }

        Ok(roots)
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

        let normalized_media_root = normalize_recycle_media_root(&entry.media_root);
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
            match crate::recycle_bin::purge_expired(&config).await {
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

                crate::recycle_bin::restore_from_recycle(&recycled_file, original_path).await?;
                let _ = tokio::fs::remove_dir_all(&entry_dir).await;
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
                    manifest,
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

                tokio::fs::remove_dir_all(&entry_dir).await.map_err(|e| {
                    AppError::Repository(format!(
                        "failed to delete recycle entry {}: {}",
                        entry_dir.display(),
                        e
                    ))
                })?;
                return Ok(true);
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
            match crate::recycle_bin::list_entries(&config, &media_root).await {
                Ok(entries) => {
                    for entry in entries {
                        let Some(library) =
                            self.resolve_recycle_entry_library(&entry, &roots).await?
                        else {
                            continue;
                        };
                        if !selected_library_ids.contains(&library.id) {
                            continue;
                        }
                        crate::recycle_bin::validate_recycle_entry_id(&entry.entry_id)?;
                        let entry_dir = config.base_path.join(&entry.entry_id);
                        match tokio::fs::remove_dir_all(&entry_dir).await {
                            Ok(()) => total += 1,
                            Err(e) => warn!(
                                path = %entry_dir.display(),
                                error = %e,
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
