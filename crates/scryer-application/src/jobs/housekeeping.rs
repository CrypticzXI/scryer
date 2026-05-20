use super::*;
use crate::events::retention::{
    OPERATIONAL_DOMAIN_EVENT_RETENTION_DAYS, operational_domain_event_types,
    user_facing_domain_event_types,
};
use tracing::{info, warn};

const RELEASE_DECISION_RETENTION_DAYS: i64 = 30;
const RELEASE_ATTEMPT_RETENTION_DAYS: i64 = 90;
const DOWNLOAD_DELETE_RETENTION_DAYS: i64 = 7;

impl AppUseCase {
    /// Resolve media root paths and their recycle configs.
    async fn resolve_all_recycle_configs(
        &self,
    ) -> Vec<(String, crate::recycle_bin::RecycleBinConfig)> {
        let mut configs = Vec::new();
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
                let config =
                    crate::recycle_bin::resolve_recycle_config(self, Some(media_root)).await;
                configs.push((media_root.to_string(), config));
            }
        }
        configs
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
    ) -> AppResult<Vec<crate::recycle_bin::RecycleEntry>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let mut all_entries = Vec::new();
        for (media_root, config) in self.resolve_all_recycle_configs().await {
            match crate::recycle_bin::list_entries(&config, &media_root).await {
                Ok(entries) => all_entries.extend(entries),
                Err(e) => {
                    info!(error = %e, media_root = %media_root, "failed to list recycle entries")
                }
            }
        }

        all_entries.sort_by(|a, b| b.manifest.recycled_at.cmp(&a.manifest.recycled_at));
        Ok(all_entries)
    }

    /// Restore a single recycled item back to its original path.
    pub async fn restore_recycled_item(
        &self,
        actor: &scryer_domain::User,
        entry_id: &str,
    ) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        for (_media_root, config) in self.resolve_all_recycle_configs().await {
            if let Some((entry_dir, manifest)) =
                crate::recycle_bin::find_entry(&config, entry_id).await?
            {
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
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        for (_media_root, config) in self.resolve_all_recycle_configs().await {
            if let Some((entry_dir, _manifest)) =
                crate::recycle_bin::find_entry(&config, entry_id).await?
            {
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
    pub async fn empty_recycle_bin(&self, actor: &scryer_domain::User) -> AppResult<u32> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let mut total = 0u32;
        for (media_root, config) in self.resolve_all_recycle_configs().await {
            match crate::recycle_bin::purge_all(&config).await {
                Ok(n) => total += n,
                Err(e) => {
                    info!(error = %e, media_root = %media_root, "failed to empty recycle bin")
                }
            }
        }
        Ok(total)
    }
}
