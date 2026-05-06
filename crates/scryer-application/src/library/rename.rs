use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use ring::digest as ring_digest;
use scryer_domain::{
    Collection, CollectionType, DomainEventPayload, Episode, ImportType, MediaFacet,
    MediaFileRenamedEventData, Title, User,
};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::activity::NotificationMediaUpdate;
use crate::domain_events::{
    created_media_update, deleted_media_update, new_title_domain_event, title_context_snapshot,
};
use crate::facet_handler::{RenameFacetSettings, rename_facet_settings};
use crate::media::release_labels::resolve_release_labels_from_analysis;
use crate::{
    AppError, AppResult, AppUseCase, CollectionUpdate, ParsedEpisodeMetadata,
    ParsedReleaseMetadata, TitleMediaFile, parse_release_metadata,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenameWriteAction {
    Noop,
    Move,
    Replace,
    Skip,
    Error,
}

impl RenameWriteAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Noop => "noop",
            Self::Move => "move",
            Self::Replace => "replace",
            Self::Skip => "skip",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenameApplyStatus {
    Applied,
    Skipped,
    Failed,
}

impl RenameApplyStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum RenameCollisionPolicy {
    #[default]
    Skip,
    Error,
    ReplaceIfBetter,
}

impl RenameCollisionPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Error => "error",
            Self::ReplaceIfBetter => "replace_if_better",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum RenameMissingMetadataPolicy {
    Skip,
    #[default]
    FallbackTitle,
}

impl RenameMissingMetadataPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::FallbackTitle => "fallback_title",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePlanItem {
    pub collection_id: Option<String>,
    pub media_file_id: Option<String>,
    pub current_path: String,
    pub proposed_path: Option<String>,
    pub normalized_filename: Option<String>,
    pub collision: bool,
    pub reason_code: String,
    pub write_action: RenameWriteAction,
    pub source_size_bytes: Option<u64>,
    pub source_mtime_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePlan {
    pub facet: MediaFacet,
    pub title_id: Option<String>,
    pub template: String,
    pub collision_policy: RenameCollisionPolicy,
    pub missing_metadata_policy: RenameMissingMetadataPolicy,
    pub fingerprint: String,
    pub total: usize,
    pub renamable: usize,
    pub noop: usize,
    pub conflicts: usize,
    pub errors: usize,
    pub items: Vec<RenamePlanItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameApplyItemResult {
    pub collection_id: Option<String>,
    pub media_file_id: Option<String>,
    pub current_path: String,
    pub proposed_path: Option<String>,
    pub final_path: Option<String>,
    pub write_action: RenameWriteAction,
    pub status: RenameApplyStatus,
    pub reason_code: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameApplyResult {
    pub plan_fingerprint: String,
    pub total: usize,
    pub applied: usize,
    pub skipped: usize,
    pub failed: usize,
    pub items: Vec<RenameApplyItemResult>,
}

#[async_trait]
pub trait LibraryRenamer: Send + Sync {
    async fn validate_targets(&self, plan: &RenamePlan) -> AppResult<()>;
    async fn apply_plan(&self, plan: &RenamePlan) -> AppResult<Vec<RenameApplyItemResult>>;
    async fn rollback(
        &self,
        applied_items: &[RenameApplyItemResult],
    ) -> AppResult<Vec<RenameApplyItemResult>>;
}

#[derive(Default)]
pub struct NullLibraryRenamer;

#[async_trait]
impl LibraryRenamer for NullLibraryRenamer {
    async fn validate_targets(&self, _plan: &RenamePlan) -> AppResult<()> {
        Err(AppError::Repository(
            "library renamer is not configured".into(),
        ))
    }

    async fn apply_plan(&self, _plan: &RenamePlan) -> AppResult<Vec<RenameApplyItemResult>> {
        Err(AppError::Repository(
            "library renamer is not configured".into(),
        ))
    }

    async fn rollback(
        &self,
        _applied_items: &[RenameApplyItemResult],
    ) -> AppResult<Vec<RenameApplyItemResult>> {
        Ok(Vec::new())
    }
}

const RENAME_TEMPLATE_KEY: &str = "rename.template";
const RENAME_COLLISION_POLICY_KEY: &str = "rename.collision_policy";
const RENAME_COLLISION_POLICY_GLOBAL_KEY: &str = "rename.collision_policy.global";
const RENAME_MISSING_METADATA_POLICY_KEY: &str = "rename.missing_metadata_policy";
const RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY: &str = "rename.missing_metadata_policy.global";
const DEFAULT_COLLISION_POLICY: RenameCollisionPolicy = RenameCollisionPolicy::Skip;
const DEFAULT_MISSING_METADATA_POLICY: RenameMissingMetadataPolicy =
    RenameMissingMetadataPolicy::FallbackTitle;

#[derive(Default)]
struct RenamePersistenceState {
    media_file_updated: bool,
}

struct RenamePersistenceFailure {
    error: AppError,
    state: RenamePersistenceState,
}

struct RenameRollbackOutcome {
    fully_restored: bool,
    detail: String,
}

struct RenamePlanSettings {
    template: String,
    collision_policy: RenameCollisionPolicy,
    missing_metadata_policy: RenameMissingMetadataPolicy,
}

impl AppUseCase {
    pub async fn preview_rename_for_title(
        &self,
        actor: &User,
        title_id: &str,
        facet: MediaFacet,
    ) -> AppResult<RenamePlan> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if title.facet != facet {
            return Err(AppError::Validation(
                "requested facet does not match title facet".into(),
            ));
        }

        let settings = self
            .read_rename_plan_settings(rename_facet_settings(&facet))
            .await?;
        self.build_rename_plan_for_titles(
            title.facet.clone(),
            std::slice::from_ref(&title),
            Some(title.id.clone()),
            settings,
        )
        .await
    }

    pub async fn preview_rename_for_facet(
        &self,
        actor: &User,
        facet: MediaFacet,
    ) -> AppResult<RenamePlan> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let settings = self
            .read_rename_plan_settings(rename_facet_settings(&facet))
            .await?;
        let mut titles = self
            .services
            .catalog
            .titles
            .list(Some(facet.clone()), None)
            .await?;
        titles.sort_by(|left, right| left.id.cmp(&right.id));
        self.build_rename_plan_for_titles(facet, &titles, None, settings)
            .await
    }

    pub async fn apply_rename_for_title(
        &self,
        actor: &User,
        title_id: &str,
        facet: MediaFacet,
        plan_fingerprint: &str,
    ) -> AppResult<RenameApplyResult> {
        let preview = self
            .preview_rename_for_title(actor, title_id, facet)
            .await?;
        self.apply_previewed_rename_plan(actor, preview, plan_fingerprint)
            .await
    }

    pub async fn apply_rename_for_facet(
        &self,
        actor: &User,
        facet: MediaFacet,
        plan_fingerprint: &str,
    ) -> AppResult<RenameApplyResult> {
        let preview = self.preview_rename_for_facet(actor, facet).await?;
        self.apply_previewed_rename_plan(actor, preview, plan_fingerprint)
            .await
    }

    pub async fn record_rename_apply_audit(
        &self,
        actor: &User,
        operation: &str,
        facet: &str,
        title_id: Option<&str>,
        idempotency_key: Option<&str>,
        result: &RenameApplyResult,
    ) -> AppResult<()> {
        if let Some(title_id) = title_id {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
        } else {
            self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
                .await?;
        }

        let now = chrono::Utc::now().to_rfc3339();
        let plan_fingerprint = result.plan_fingerprint.clone();
        let progress_json = serde_json::json!({
            "operation": operation,
            "facet": facet,
            "title_id": title_id,
            "idempotency_key": idempotency_key,
            "plan_fingerprint": plan_fingerprint.clone(),
            "total": result.total,
            "applied": result.applied,
            "skipped": result.skipped,
            "failed": result.failed,
        })
        .to_string();

        let _ = self
            .services
            .workflow
            .workflow_operations
            .create_workflow_operation(
                operation.to_string(),
                "completed".to_string(),
                Some(actor.id.clone()),
                Some(progress_json),
                Some(now.clone()),
                Some(now),
            )
            .await?;

        let source_ref = if let Some(key) = idempotency_key {
            format!("{operation}:{key}")
        } else if let Some(title_id) = title_id {
            format!("{operation}:title:{title_id}:{plan_fingerprint}")
        } else {
            format!("{operation}:facet:{facet}:{plan_fingerprint}")
        };
        let payload_json = serde_json::to_string(result).unwrap_or_else(|_| {
            "{\"error\":\"failed_to_serialize_rename_apply_result\"}".to_string()
        });

        let _ = self
            .services
            .workflow
            .imports
            .queue_import_request(
                "scryer_rename".to_string(),
                source_ref,
                ImportType::RenameApplyResult.as_str().to_string(),
                payload_json,
            )
            .await?;

        Ok(())
    }

    async fn read_rename_plan_settings(
        &self,
        facet_settings: RenameFacetSettings,
    ) -> AppResult<RenamePlanSettings> {
        Ok(RenamePlanSettings {
            template: self.read_rename_template(facet_settings).await?,
            collision_policy: self.read_collision_policy(facet_settings).await?,
            missing_metadata_policy: self.read_missing_metadata_policy(facet_settings).await?,
        })
    }

    async fn apply_previewed_rename_plan(
        &self,
        actor: &User,
        preview: RenamePlan,
        plan_fingerprint: &str,
    ) -> AppResult<RenameApplyResult> {
        if preview.fingerprint != plan_fingerprint {
            return Err(AppError::Validation("rename_stale_plan".into()));
        }

        self.apply_rename_plan(actor, preview).await
    }

    async fn apply_rename_plan(
        &self,
        actor: &User,
        preview: RenamePlan,
    ) -> AppResult<RenameApplyResult> {
        self.services
            .library
            .library_renamer
            .validate_targets(&preview)
            .await?;

        let mut item_results = self
            .services
            .library
            .library_renamer
            .apply_plan(&preview)
            .await?;
        let mut applied = 0usize;
        let mut skipped = 0usize;
        let mut failed = 0usize;

        for item in &mut item_results {
            match item.status {
                RenameApplyStatus::Applied => {
                    if let Some(final_path) = item.final_path.clone()
                        && let Err(failure) =
                            self.persist_rename_item_paths(item, &final_path).await
                    {
                        let rollback = self
                            .rollback_rename_item_after_db_failure(item, &failure.state)
                            .await;

                        item.status = RenameApplyStatus::Failed;
                        item.reason_code = "db_update_failed".into();
                        item.error_message =
                            Some(format!("{}; {}", failure.error, rollback.detail));
                        if rollback.fully_restored {
                            item.final_path = Some(item.current_path.clone());
                        }
                        failed += 1;
                        continue;
                    }
                    applied += 1;
                }
                RenameApplyStatus::Skipped => {
                    skipped += 1;
                }
                RenameApplyStatus::Failed => {
                    failed += 1;
                }
            }
        }

        let result = RenameApplyResult {
            plan_fingerprint: preview.fingerprint.clone(),
            total: item_results.len(),
            applied,
            skipped,
            failed,
            items: item_results,
        };

        self.emit_rename_notifications(actor, &result.items).await;

        Ok(result)
    }

    async fn emit_rename_notifications(&self, actor: &User, items: &[RenameApplyItemResult]) {
        let mut grouped: HashMap<String, (Title, Vec<NotificationMediaUpdate>, Vec<String>)> =
            HashMap::new();
        let mut cached_episode_ids_by_file: HashMap<String, Vec<String>> = HashMap::new();

        for item in items {
            if !matches!(item.status, RenameApplyStatus::Applied) {
                continue;
            }

            let Some(final_path) = item.final_path.clone() else {
                continue;
            };

            let title = match self.resolve_title_for_rename_item(item).await {
                Ok(Some(title)) => title,
                Ok(None) => continue,
                Err(error) => {
                    warn!(
                        error = %error,
                        current_path = item.current_path.as_str(),
                        "failed to resolve title for rename notification"
                    );
                    continue;
                }
            };

            let episode_ids = if let Some(media_file_id) = item.media_file_id.as_deref() {
                if let Some(cached) = cached_episode_ids_by_file.get(media_file_id) {
                    cached.clone()
                } else {
                    let ids = self
                        .services
                        .library
                        .media_files
                        .list_media_files_for_title(&title.id)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|media_file| media_file.id == media_file_id)
                        .filter_map(|media_file| media_file.episode_id)
                        .collect::<Vec<_>>();
                    cached_episode_ids_by_file.insert(media_file_id.to_string(), ids.clone());
                    ids
                }
            } else {
                Vec::new()
            };

            let entry = grouped
                .entry(title.id.clone())
                .or_insert_with(|| (title.clone(), Vec::new(), Vec::new()));
            entry
                .1
                .push(NotificationMediaUpdate::deleted(item.current_path.clone()));
            entry.1.push(NotificationMediaUpdate::created(final_path));
            for episode_id in episode_ids {
                if !entry.2.contains(&episode_id) {
                    entry.2.push(episode_id);
                }
            }
        }

        for (_title_id, (title, updates, episode_ids)) in grouped {
            if updates.is_empty() {
                continue;
            }

            let renamed_files = updates
                .iter()
                .filter(|u| u.update_type == "created")
                .count();
            let domain_updates = updates
                .iter()
                .map(|update| match update.update_type {
                    "deleted" => deleted_media_update(update.path.clone()),
                    _ => created_media_update(update.path.clone()),
                })
                .collect();
            if let Err(error) = self
                .append_domain_event(new_title_domain_event(
                    Some(actor.id.clone()),
                    &title,
                    DomainEventPayload::MediaFileRenamed(MediaFileRenamedEventData {
                        title: title_context_snapshot(&title),
                        media_updates: domain_updates,
                        renamed_count: renamed_files as i32,
                        episode_ids,
                    }),
                ))
                .await
            {
                warn!(
                    error = %error,
                    title = title.name.as_str(),
                    "failed to append media file renamed domain event"
                );
            }
        }
    }

    async fn resolve_title_for_rename_item(
        &self,
        item: &RenameApplyItemResult,
    ) -> AppResult<Option<Title>> {
        let title_id = if let Some(media_file_id) = item.media_file_id.as_deref() {
            self.services
                .library
                .media_files
                .get_media_file_by_id(media_file_id)
                .await?
                .map(|file| file.title_id)
        } else if let Some(collection_id) = item.collection_id.as_deref() {
            self.services
                .catalog
                .shows
                .get_collection_by_id(collection_id)
                .await?
                .map(|collection| collection.title_id)
        } else {
            None
        };

        match title_id {
            Some(title_id) => self.services.catalog.titles.get_by_id(&title_id).await,
            None => Ok(None),
        }
    }

    async fn persist_rename_item_paths(
        &self,
        item: &RenameApplyItemResult,
        final_path: &str,
    ) -> Result<(), RenamePersistenceFailure> {
        let mut state = RenamePersistenceState::default();

        if let Some(media_file_id) = item.media_file_id.as_deref()
            && let Err(error) = self
                .services
                .library
                .media_files
                .update_media_file_path(media_file_id, final_path)
                .await
        {
            return Err(RenamePersistenceFailure { error, state });
        } else if item.media_file_id.is_some() {
            state.media_file_updated = true;
        }

        if let Some(collection_id) = item.collection_id.as_deref()
            && let Err(error) = self
                .services
                .catalog
                .shows
                .update_collection(
                    collection_id,
                    CollectionUpdate {
                        ordered_path: Some(final_path.to_string()),
                        ..Default::default()
                    },
                )
                .await
        {
            return Err(RenamePersistenceFailure { error, state });
        }

        Ok(())
    }

    async fn rollback_rename_item_after_db_failure(
        &self,
        item: &RenameApplyItemResult,
        state: &RenamePersistenceState,
    ) -> RenameRollbackOutcome {
        let mut details = Vec::new();
        let mut fully_restored = true;
        let mut filesystem_restored = false;

        match item.write_action {
            RenameWriteAction::Move => match self
                .services
                .library
                .library_renamer
                .rollback(std::slice::from_ref(item))
                .await
            {
                Ok(_) => {
                    filesystem_restored = true;
                }
                Err(error) => {
                    fully_restored = false;
                    details.push(format!("filesystem rollback failed: {error}"));
                }
            },
            _ => {
                fully_restored = false;
                details.push("filesystem rollback unavailable for this write action".to_string());
            }
        }

        if filesystem_restored
            && state.media_file_updated
            && let Some(media_file_id) = item.media_file_id.as_deref()
            && let Err(error) = self
                .services
                .library
                .media_files
                .update_media_file_path(media_file_id, &item.current_path)
                .await
        {
            fully_restored = false;
            details.push(format!("media file rollback failed: {error}"));
        }

        if details.is_empty() {
            RenameRollbackOutcome {
                fully_restored,
                detail: "rollback succeeded".to_string(),
            }
        } else {
            RenameRollbackOutcome {
                fully_restored,
                detail: format!("rollback failed: {}", details.join("; ")),
            }
        }
    }

    async fn build_rename_plan_for_titles(
        &self,
        facet: MediaFacet,
        titles: &[Title],
        title_id: Option<String>,
        settings: RenamePlanSettings,
    ) -> AppResult<RenamePlan> {
        let mut planned_targets = HashSet::new();
        let mut items = Vec::new();
        for title in titles {
            let mut title_items = self
                .build_rename_plan_items_for_title(
                    title,
                    &settings.template,
                    &settings.collision_policy,
                    &settings.missing_metadata_policy,
                    &mut planned_targets,
                )
                .await?;
            items.append(&mut title_items);
        }

        Ok(build_rename_plan_from_items(
            facet,
            title_id,
            settings.template,
            settings.collision_policy,
            settings.missing_metadata_policy,
            items,
        ))
    }

    async fn build_rename_plan_items_for_title(
        &self,
        title: &Title,
        template: &str,
        collision_policy: &RenameCollisionPolicy,
        missing_metadata_policy: &RenameMissingMetadataPolicy,
        planned_targets: &mut HashSet<String>,
    ) -> AppResult<Vec<RenamePlanItem>> {
        let collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await?;
        let media_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await?;

        let items = match title.facet.clone() {
            MediaFacet::Movie => build_movie_rename_plan_items(
                title,
                collections,
                media_files,
                template,
                collision_policy,
                missing_metadata_policy,
                planned_targets,
            ),
            MediaFacet::Series | MediaFacet::Anime => {
                let episodes = self
                    .services
                    .catalog
                    .shows
                    .list_episodes_for_title(&title.id)
                    .await?;

                build_series_rename_plan_items_from_media_files(
                    title,
                    collections,
                    episodes,
                    media_files,
                    template,
                    collision_policy,
                    missing_metadata_policy,
                    planned_targets,
                )
            }
        };

        self.normalize_existing_rename_collisions(items).await
    }

    async fn normalize_existing_rename_collisions(
        &self,
        items: Vec<RenamePlanItem>,
    ) -> AppResult<Vec<RenamePlanItem>> {
        let mut collection_cache = HashMap::<String, Option<Collection>>::new();
        let mut media_file_cache = HashMap::<String, Option<TitleMediaFile>>::new();
        let mut out = Vec::with_capacity(items.len());

        for mut item in items {
            let Some(proposed_path) = item.proposed_path.clone() else {
                out.push(item);
                continue;
            };

            if proposed_path == item.current_path {
                out.push(item);
                continue;
            }

            let destination_exists_on_disk = Path::new(&proposed_path).exists();

            let tracked_media_file = if let Some(existing) = media_file_cache.get(&proposed_path) {
                existing.clone()
            } else {
                let loaded = self
                    .services
                    .library
                    .media_files
                    .get_media_file_by_path(&proposed_path)
                    .await?;
                media_file_cache.insert(proposed_path.clone(), loaded.clone());
                loaded
            };
            let tracked_collection = if let Some(existing) = collection_cache.get(&proposed_path) {
                existing.clone()
            } else {
                let loaded = self
                    .services
                    .catalog
                    .shows
                    .get_collection_by_ordered_path(&proposed_path)
                    .await?;
                collection_cache.insert(proposed_path.clone(), loaded.clone());
                loaded
            };

            let tracked_media_conflict = tracked_media_file.as_ref().is_some_and(|media_file| {
                item.media_file_id.as_deref() != Some(media_file.id.as_str())
            });
            let tracked_collection_conflict =
                tracked_collection.as_ref().is_some_and(|collection| {
                    item.collection_id.as_deref() != Some(collection.id.as_str())
                });

            if tracked_media_conflict || tracked_collection_conflict {
                item.collision = true;
                item.reason_code = "collision_existing_tracked".into();
                item.write_action = RenameWriteAction::Error;
            } else if !destination_exists_on_disk {
                out.push(item);
                continue;
            } else if matches!(item.write_action, RenameWriteAction::Replace) {
                item.collision = true;
                item.reason_code = "collision_existing".into();
                item.write_action = RenameWriteAction::Error;
            }

            out.push(item);
        }

        Ok(out)
    }

    async fn read_rename_template(&self, facet_settings: RenameFacetSettings) -> AppResult<String> {
        if let Some(scoped) = self
            .read_setting_string_value(RENAME_TEMPLATE_KEY, Some(facet_settings.scope_id))
            .await?
            .filter(|value| !value.trim().is_empty())
        {
            return Ok(scoped);
        }

        if let Some(global) = self
            .read_setting_string_value(facet_settings.template_key, None)
            .await?
            .filter(|value| !value.trim().is_empty())
        {
            return Ok(global);
        }

        Ok(facet_settings.default_template.to_string())
    }

    async fn read_collision_policy(
        &self,
        facet_settings: RenameFacetSettings,
    ) -> AppResult<RenameCollisionPolicy> {
        self.read_rename_policy(
            facet_settings,
            RENAME_COLLISION_POLICY_KEY,
            RENAME_COLLISION_POLICY_GLOBAL_KEY,
            facet_settings.collision_policy_key,
            parse_collision_policy,
            DEFAULT_COLLISION_POLICY,
        )
        .await
    }

    async fn read_missing_metadata_policy(
        &self,
        facet_settings: RenameFacetSettings,
    ) -> AppResult<RenameMissingMetadataPolicy> {
        self.read_rename_policy(
            facet_settings,
            RENAME_MISSING_METADATA_POLICY_KEY,
            RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY,
            facet_settings.missing_metadata_policy_key,
            parse_missing_metadata_policy,
            DEFAULT_MISSING_METADATA_POLICY,
        )
        .await
    }

    async fn read_rename_policy<T>(
        &self,
        facet_settings: RenameFacetSettings,
        scoped_key: &str,
        global_key: &str,
        handler_key: &str,
        parse: impl Fn(&str) -> Option<T>,
        default: T,
    ) -> AppResult<T> {
        let scoped = self
            .read_setting_string_value(scoped_key, Some(facet_settings.scope_id))
            .await?;
        if let Some(value) = scoped
            && let Some(parsed) = parse(&value)
        {
            return Ok(parsed);
        }

        let global = self.read_setting_string_value(global_key, None).await?;
        if let Some(value) = global
            && let Some(parsed) = parse(&value)
        {
            return Ok(parsed);
        }

        let handler_value = self.read_setting_string_value(handler_key, None).await?;
        if let Some(value) = handler_value
            && let Some(parsed) = parse(&value)
        {
            return Ok(parsed);
        }

        Ok(default)
    }
}

fn parse_collision_policy(raw: &str) -> Option<RenameCollisionPolicy> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "skip" => Some(RenameCollisionPolicy::Skip),
        "error" => Some(RenameCollisionPolicy::Error),
        "replace_if_better" => Some(RenameCollisionPolicy::ReplaceIfBetter),
        _ => None,
    }
}

fn parse_missing_metadata_policy(raw: &str) -> Option<RenameMissingMetadataPolicy> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "skip" => Some(RenameMissingMetadataPolicy::Skip),
        "fallback_title" => Some(RenameMissingMetadataPolicy::FallbackTitle),
        _ => None,
    }
}

fn build_movie_rename_plan_items(
    title: &Title,
    mut collections: Vec<Collection>,
    media_files: Vec<TitleMediaFile>,
    template: &str,
    collision_policy: &RenameCollisionPolicy,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
    planned_targets: &mut HashSet<String>,
) -> Vec<RenamePlanItem> {
    collections.sort_by(|left, right| left.id.cmp(&right.id));
    let media_files_by_path = media_files.into_iter().fold(
        HashMap::<String, TitleMediaFile>::new(),
        |mut acc, media_file| {
            acc.entry(media_file.file_path.clone())
                .or_insert(media_file);
            acc
        },
    );

    collections
        .into_iter()
        .map(|collection| {
            let matched_media_file = collection
                .ordered_path
                .as_deref()
                .and_then(|path| media_files_by_path.get(path));
            let mut item = build_movie_rename_plan_item(
                title,
                &collection,
                matched_media_file,
                template,
                collision_policy,
                missing_metadata_policy,
                planned_targets,
            );
            if item.media_file_id.is_none() {
                item.media_file_id = matched_media_file.map(|media_file| media_file.id.clone());
            }
            item
        })
        .collect()
}

pub fn render_rename_template(template: &str, tokens: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let chars: Vec<char> = template.chars().collect();
    let mut cursor = 0usize;

    while cursor < chars.len() {
        let ch = chars[cursor];
        if ch != '{' {
            out.push(ch);
            cursor += 1;
            continue;
        }

        if let Some(end) = chars[cursor + 1..].iter().position(|c| *c == '}') {
            let end_index = cursor + 1 + end;
            let token_spec: String = chars[cursor + 1..end_index].iter().collect();
            out.push_str(&resolve_template_token(tokens, token_spec.trim()));
            cursor = end_index + 1;
            continue;
        }

        out.push(ch);
        cursor += 1;
    }

    sanitize_filesystem_component(&out)
}

pub fn build_rename_plan_fingerprint(
    items: &[RenamePlanItem],
    template: &str,
    collision_policy: &RenameCollisionPolicy,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
) -> String {
    let bytes = serde_json::to_vec(&(
        template,
        collision_policy.as_str(),
        missing_metadata_policy.as_str(),
        items,
    ))
    .unwrap_or_default();
    let hash = ring_digest::digest(&ring_digest::SHA256, &bytes);
    crate::to_hex(hash.as_ref())
}

struct GroupedTitleMediaFile {
    file: TitleMediaFile,
    episode_ids: Vec<String>,
}

struct ResolvedSeriesRenameMetadata {
    collection_id: Option<String>,
    season: String,
    season_order: String,
    episode: String,
    absolute_episode: String,
    episode_title: String,
}

#[derive(Clone)]
struct RenamePlanItemIds {
    collection_id: Option<String>,
    media_file_id: Option<String>,
}

struct RenamePlanSource {
    current_path: String,
    current_file: PathBuf,
    extension: String,
    source_size_bytes: Option<u64>,
    source_mtime_unix_ms: Option<i64>,
}

struct RenameCommonTokens {
    title: String,
    year: String,
    quality: String,
    source: String,
    video_codec: String,
    audio_codec: String,
    audio_channels: String,
    group: String,
    extension: String,
}

struct ResolvedRenameCommonMetadata {
    common: RenameCommonTokens,
    edition: String,
}

impl RenamePlanSource {
    fn build_item(
        &self,
        item_ids: RenamePlanItemIds,
        proposed_path: Option<String>,
        normalized_filename: Option<String>,
        collision: bool,
        reason_code: &'static str,
        write_action: RenameWriteAction,
    ) -> RenamePlanItem {
        rename_plan_item(
            item_ids,
            self.current_path.clone(),
            proposed_path,
            normalized_filename,
            collision,
            reason_code,
            write_action,
            self.source_size_bytes,
            self.source_mtime_unix_ms,
        )
    }
}

fn rename_plan_item(
    item_ids: RenamePlanItemIds,
    current_path: String,
    proposed_path: Option<String>,
    normalized_filename: Option<String>,
    collision: bool,
    reason_code: &'static str,
    write_action: RenameWriteAction,
    source_size_bytes: Option<u64>,
    source_mtime_unix_ms: Option<i64>,
) -> RenamePlanItem {
    RenamePlanItem {
        collection_id: item_ids.collection_id,
        media_file_id: item_ids.media_file_id,
        current_path,
        proposed_path,
        normalized_filename,
        collision,
        reason_code: reason_code.into(),
        write_action,
        source_size_bytes,
        source_mtime_unix_ms,
    }
}

fn prepare_rename_plan_source(
    item_ids: RenamePlanItemIds,
    current_path: Option<String>,
) -> Result<RenamePlanSource, Box<RenamePlanItem>> {
    let current_path = current_path.unwrap_or_default();
    if current_path.trim().is_empty() {
        return Err(Box::new(rename_plan_item(
            item_ids,
            current_path,
            None,
            None,
            false,
            "no_source_path",
            RenameWriteAction::Skip,
            None,
            None,
        )));
    }

    let current_file = PathBuf::from(&current_path);
    let source_metadata = std::fs::metadata(&current_file).ok();
    let source_size_bytes = source_metadata.as_ref().map(|meta| meta.len());
    let source_mtime_unix_ms = source_metadata
        .as_ref()
        .and_then(|meta| meta.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok());
    let source = RenamePlanSource {
        extension: current_file
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_default(),
        current_path,
        current_file,
        source_size_bytes,
        source_mtime_unix_ms,
    };

    if source_metadata.as_ref().is_none_or(|meta| !meta.is_file()) {
        return Err(Box::new(source.build_item(
            item_ids,
            None,
            None,
            false,
            "source_not_file",
            RenameWriteAction::Error,
        )));
    }

    Ok(source)
}

fn insert_common_rename_tokens(tokens: &mut BTreeMap<String, String>, common: RenameCommonTokens) {
    tokens.insert("title".to_string(), common.title);
    tokens.insert("year".to_string(), common.year);
    tokens.insert("quality".to_string(), common.quality);
    tokens.insert("source".to_string(), common.source);
    tokens.insert("video_codec".to_string(), common.video_codec);
    tokens.insert("audio_codec".to_string(), common.audio_codec);
    tokens.insert("audio_channels".to_string(), common.audio_channels);
    tokens.insert("group".to_string(), common.group);
    tokens.insert("ext".to_string(), common.extension);
}

fn resolved_analysis_labels_for_media_file(
    media_file: &TitleMediaFile,
) -> crate::media::release_labels::ResolvedAnalysisReleaseLabels {
    resolve_release_labels_from_analysis(
        media_file.video_height,
        media_file.video_codec.as_deref(),
        media_file.audio_codec.as_deref(),
        media_file.audio_profile.as_deref(),
        media_file.audio_channels,
        &media_file.audio_streams,
    )
}

fn resolve_rename_common_metadata(
    media_file: Option<&TitleMediaFile>,
    parsed_current: &ParsedReleaseMetadata,
    title_token: &str,
    year_token: Option<&str>,
    extension: &str,
) -> ResolvedRenameCommonMetadata {
    let analyzed = media_file
        .map(resolved_analysis_labels_for_media_file)
        .unwrap_or_default();

    let quality = analyzed
        .quality
        .or_else(|| media_file.and_then(|file| non_empty_owned(file.quality_label.clone())))
        .or_else(|| parsed_current.quality.clone())
        .unwrap_or_default();
    let source = media_file
        .and_then(|file| non_empty_owned(file.source_type.clone()))
        .or_else(|| parsed_current.source.clone())
        .unwrap_or_default();
    let video_codec = analyzed
        .video_codec
        .or_else(|| media_file.and_then(|file| non_empty_owned(file.video_codec_parsed.clone())))
        .or_else(|| parsed_current.video_codec.clone())
        .unwrap_or_default();
    let audio_codec = analyzed
        .audio_codec
        .or_else(|| media_file.and_then(|file| non_empty_owned(file.audio_codec_parsed.clone())))
        .or_else(|| parsed_current.audio.clone())
        .unwrap_or_default();
    let audio_channels = analyzed
        .audio_channels
        .or_else(|| media_file.and_then(|file| non_empty_owned(file.audio_channels_parsed.clone())))
        .or_else(|| parsed_current.audio_channels.clone())
        .unwrap_or_default();
    let group = media_file
        .and_then(|file| non_empty_owned(file.release_group.clone()))
        .or_else(|| parsed_current.release_group.clone())
        .unwrap_or_default();
    let edition = media_file
        .and_then(|file| non_empty_owned(file.edition.clone()))
        .or_else(|| {
            parsed_current
                .parse_hints
                .iter()
                .find(|hint| hint.to_ascii_lowercase().contains("edition"))
                .cloned()
        })
        .unwrap_or_default();

    ResolvedRenameCommonMetadata {
        common: RenameCommonTokens {
            title: title_token.to_string(),
            year: year_token.unwrap_or_default().to_string(),
            quality,
            source,
            video_codec,
            audio_codec,
            audio_channels,
            group,
            extension: extension.to_string(),
        },
        edition,
    }
}

fn resolve_rendered_rename_filename(
    source: &RenamePlanSource,
    item_ids: RenamePlanItemIds,
    template: &str,
    tokens: &BTreeMap<String, String>,
    fallback_title: &str,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
) -> Result<String, Box<RenamePlanItem>> {
    let mut rendered = render_rename_template(template, tokens);
    if rendered.is_empty() {
        if matches!(missing_metadata_policy, RenameMissingMetadataPolicy::Skip) {
            return Err(Box::new(source.build_item(
                item_ids,
                None,
                None,
                false,
                "missing_metadata",
                RenameWriteAction::Skip,
            )));
        }
        rendered = fallback_title.to_string();
    }

    if !source.extension.is_empty()
        && !rendered
            .to_ascii_lowercase()
            .ends_with(&format!(".{}", source.extension))
    {
        rendered = format!("{rendered}.{}", source.extension);
    }

    Ok(rendered)
}

fn finalize_rename_plan_item(
    source: &RenamePlanSource,
    item_ids: RenamePlanItemIds,
    rendered: String,
    collision_policy: &RenameCollisionPolicy,
    planned_targets: &mut HashSet<String>,
) -> RenamePlanItem {
    let parent = source
        .current_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let proposed_path_str = parent.join(&rendered).to_string_lossy().to_string();

    if proposed_path_str == source.current_path {
        return source.build_item(
            item_ids,
            Some(proposed_path_str),
            Some(rendered),
            false,
            "same_path",
            RenameWriteAction::Noop,
        );
    }

    if !planned_targets.insert(proposed_path_str.clone()) {
        return source.build_item(
            item_ids,
            Some(proposed_path_str),
            Some(rendered),
            true,
            "collision_within_plan",
            RenameWriteAction::Skip,
        );
    }

    if Path::new(&proposed_path_str).exists() {
        return source.build_item(
            item_ids,
            Some(proposed_path_str),
            Some(rendered),
            true,
            "collision_existing",
            existing_collision_write_action(collision_policy),
        );
    }

    source.build_item(
        item_ids,
        Some(proposed_path_str),
        Some(rendered),
        false,
        "rename_move",
        RenameWriteAction::Move,
    )
}

fn existing_collision_write_action(collision_policy: &RenameCollisionPolicy) -> RenameWriteAction {
    match collision_policy {
        RenameCollisionPolicy::Skip => RenameWriteAction::Skip,
        RenameCollisionPolicy::Error | RenameCollisionPolicy::ReplaceIfBetter => {
            RenameWriteAction::Error
        }
    }
}

pub(crate) fn build_series_rename_plan_items_from_media_files(
    title: &Title,
    mut collections: Vec<Collection>,
    episodes: Vec<Episode>,
    media_files: Vec<TitleMediaFile>,
    template: &str,
    collision_policy: &RenameCollisionPolicy,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
    planned_targets: &mut HashSet<String>,
) -> Vec<RenamePlanItem> {
    collections.sort_by(|left, right| left.id.cmp(&right.id));

    let collections_by_id = collections
        .iter()
        .cloned()
        .map(|collection| (collection.id.clone(), collection))
        .collect::<HashMap<_, _>>();
    let episodes_by_id = episodes
        .into_iter()
        .map(|episode| (episode.id.clone(), episode))
        .collect::<HashMap<_, _>>();

    let mut grouped_files = group_title_media_files(media_files);
    grouped_files.sort_by(|left, right| {
        left.file
            .file_path
            .cmp(&right.file.file_path)
            .then_with(|| left.file.id.cmp(&right.file.id))
    });

    grouped_files
        .into_iter()
        .map(|source| {
            build_series_media_file_rename_plan_item(
                title,
                &collections,
                &collections_by_id,
                &episodes_by_id,
                source,
                template,
                collision_policy,
                missing_metadata_policy,
                planned_targets,
            )
        })
        .collect()
}

fn group_title_media_files(media_files: Vec<TitleMediaFile>) -> Vec<GroupedTitleMediaFile> {
    let mut grouped: Vec<GroupedTitleMediaFile> = Vec::new();
    let mut indexes: HashMap<String, usize> = HashMap::new();

    for media_file in media_files {
        if let Some(index) = indexes.get(&media_file.id).copied() {
            if let Some(episode_id) = media_file.episode_id.as_ref()
                && !grouped[index]
                    .episode_ids
                    .iter()
                    .any(|value| value == episode_id)
            {
                grouped[index].episode_ids.push(episode_id.clone());
            }
            continue;
        }

        let episode_ids = media_file
            .episode_id
            .clone()
            .into_iter()
            .collect::<Vec<_>>();
        indexes.insert(media_file.id.clone(), grouped.len());
        grouped.push(GroupedTitleMediaFile {
            file: media_file,
            episode_ids,
        });
    }

    grouped
}

fn build_series_media_file_rename_plan_item(
    title: &Title,
    collections: &[Collection],
    collections_by_id: &HashMap<String, Collection>,
    episodes_by_id: &HashMap<String, Episode>,
    source: GroupedTitleMediaFile,
    template: &str,
    collision_policy: &RenameCollisionPolicy,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
    planned_targets: &mut HashSet<String>,
) -> RenamePlanItem {
    let source_item_ids = RenamePlanItemIds {
        collection_id: None,
        media_file_id: Some(source.file.id.clone()),
    };
    let source_file = match prepare_rename_plan_source(
        source_item_ids.clone(),
        Some(source.file.file_path.clone()),
    ) {
        Ok(source_file) => source_file,
        Err(item) => return *item,
    };

    let current_stem = source_file
        .current_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let parsed = parse_release_metadata(current_stem);
    let rename_metadata = resolve_series_rename_metadata(
        collections,
        collections_by_id,
        episodes_by_id,
        &source,
        &parsed,
    );
    let (title_token, year_token) = split_title_and_year_hint(&title.name);
    let extension = source_file.extension.clone();
    let common = resolve_rename_common_metadata(
        Some(&source.file),
        &parsed,
        &title_token,
        year_token.as_deref(),
        &extension,
    );

    let mut tokens = BTreeMap::new();
    insert_common_rename_tokens(&mut tokens, common.common);
    tokens.insert("season".to_string(), rename_metadata.season.clone());
    tokens.insert(
        "season_order".to_string(),
        rename_metadata.season_order.clone(),
    );
    tokens.insert("episode".to_string(), rename_metadata.episode.clone());
    tokens.insert(
        "absolute_episode".to_string(),
        rename_metadata.absolute_episode.clone(),
    );
    tokens.insert(
        "episode_title".to_string(),
        rename_metadata.episode_title.clone(),
    );

    let item_ids = RenamePlanItemIds {
        collection_id: rename_metadata.collection_id.clone(),
        media_file_id: source_item_ids.media_file_id,
    };
    let rendered = match resolve_rendered_rename_filename(
        &source_file,
        item_ids.clone(),
        template,
        &tokens,
        &title_token,
        missing_metadata_policy,
    ) {
        Ok(rendered) => rendered,
        Err(item) => return *item,
    };

    finalize_rename_plan_item(
        &source_file,
        item_ids,
        rendered,
        collision_policy,
        planned_targets,
    )
}

fn resolve_series_rename_metadata(
    collections: &[Collection],
    collections_by_id: &HashMap<String, Collection>,
    episodes_by_id: &HashMap<String, Episode>,
    source: &GroupedTitleMediaFile,
    parsed: &ParsedReleaseMetadata,
) -> ResolvedSeriesRenameMetadata {
    if source.episode_ids.is_empty()
        && let Some(collection) = collections.iter().find(|collection| {
            collection.collection_type == CollectionType::Interstitial
                && collection.ordered_path.as_deref() == Some(source.file.file_path.as_str())
        })
    {
        let (season, episode) =
            parse_interstitial_season_episode(collection.interstitial_season_episode.as_deref())
                .unwrap_or_else(|| ("0".to_string(), "1".to_string()));

        return ResolvedSeriesRenameMetadata {
            collection_id: Some(collection.id.clone()),
            season_order: non_empty_owned(collection.narrative_order.clone())
                .or_else(|| non_empty_string(&collection.collection_index))
                .unwrap_or_else(|| season.clone()),
            absolute_episode: parsed
                .episode
                .as_ref()
                .and_then(|episode_meta| episode_meta.absolute_episode)
                .map(|value| format!("{value:03}"))
                .unwrap_or_else(|| episode.clone()),
            episode_title: collection
                .interstitial_movie
                .as_ref()
                .map(|movie| movie.name.clone())
                .unwrap_or_default(),
            season,
            episode,
        };
    }

    let linked_episodes =
        select_sorted_episodes(&source.episode_ids, episodes_by_id, collections_by_id);
    if let Some(primary_episode) = linked_episodes.first().copied() {
        let collection = primary_episode
            .collection_id
            .as_deref()
            .and_then(|collection_id| collections_by_id.get(collection_id));
        let parsed_episode = parsed.episode.as_ref();
        let season = non_empty_owned(primary_episode.season_number.clone())
            .or_else(|| collection.and_then(|value| non_empty_string(&value.collection_index)))
            .or_else(|| {
                parsed_episode
                    .and_then(|value| value.season)
                    .map(|value| value.to_string())
            })
            .unwrap_or_default();
        let episode = format_number_token(collect_episode_numbers(&linked_episodes), 2, false)
            .or_else(|| non_empty_owned(primary_episode.episode_number.clone()))
            .or_else(|| parsed_episode.and_then(parsed_episode_token))
            .unwrap_or_default();

        return ResolvedSeriesRenameMetadata {
            collection_id: None,
            season_order: collection
                .and_then(|value| non_empty_owned(value.narrative_order.clone()))
                .or_else(|| collection.and_then(|value| non_empty_string(&value.collection_index)))
                .or_else(|| non_empty_owned(primary_episode.season_number.clone()))
                .unwrap_or_else(|| season.clone()),
            absolute_episode: format_number_token(
                collect_absolute_episode_numbers(&linked_episodes),
                3,
                true,
            )
            .or_else(|| normalize_absolute_episode_token(primary_episode.absolute_number.clone()))
            .or_else(|| parsed_episode.and_then(parsed_absolute_episode_token))
            .unwrap_or_else(|| episode.clone()),
            episode_title: join_episode_titles(&linked_episodes).unwrap_or_default(),
            season,
            episode,
        };
    }

    let parsed_episode = parsed.episode.as_ref();
    let season = parsed_episode
        .and_then(|value| value.season)
        .map(|value| value.to_string())
        .unwrap_or_default();
    let episode = parsed_episode
        .and_then(parsed_episode_token)
        .unwrap_or_default();

    ResolvedSeriesRenameMetadata {
        collection_id: None,
        season_order: if season.is_empty() {
            String::new()
        } else {
            season.clone()
        },
        absolute_episode: parsed_episode
            .and_then(parsed_absolute_episode_token)
            .unwrap_or_else(|| episode.clone()),
        episode_title: String::new(),
        season,
        episode,
    }
}

fn select_sorted_episodes<'a>(
    episode_ids: &[String],
    episodes_by_id: &'a HashMap<String, Episode>,
    collections_by_id: &HashMap<String, Collection>,
) -> Vec<&'a Episode> {
    let mut episodes = episode_ids
        .iter()
        .filter_map(|episode_id| episodes_by_id.get(episode_id))
        .collect::<Vec<_>>();
    episodes.sort_by_key(|episode| episode_sort_key(episode, collections_by_id));
    episodes
}

fn collect_episode_numbers(episodes: &[&Episode]) -> Vec<u32> {
    episodes
        .iter()
        .filter_map(|episode| parse_sort_number(episode.episode_number.as_deref()))
        .collect()
}

fn collect_absolute_episode_numbers(episodes: &[&Episode]) -> Vec<u32> {
    episodes
        .iter()
        .filter_map(|episode| parse_sort_number(episode.absolute_number.as_deref()))
        .collect()
}

fn join_episode_titles(episodes: &[&Episode]) -> Option<String> {
    let mut seen = HashSet::new();
    let mut titles = Vec::new();

    for episode in episodes {
        let Some(title) = episode
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };

        let normalized = title.to_ascii_lowercase();
        if seen.insert(normalized) {
            titles.push(title.to_string());
        }
    }

    if titles.is_empty() {
        None
    } else {
        Some(titles.join(" + "))
    }
}

fn format_number_token(mut numbers: Vec<u32>, width: usize, pad_single: bool) -> Option<String> {
    if numbers.is_empty() {
        return None;
    }

    numbers.sort_unstable();
    numbers.dedup();

    if numbers.len() == 1 {
        let value = numbers[0];
        return Some(if pad_single {
            format!("{value:0width$}")
        } else {
            value.to_string()
        });
    }

    Some(
        numbers
            .into_iter()
            .map(|value| format!("{value:0width$}"))
            .collect::<Vec<_>>()
            .join("-"),
    )
}

fn parsed_episode_token(parsed_episode: &ParsedEpisodeMetadata) -> Option<String> {
    if !parsed_episode.episode_numbers.is_empty() {
        format_number_token(parsed_episode.episode_numbers.clone(), 2, false)
    } else {
        parsed_episode
            .first_episode()
            .map(|value| value.to_string())
    }
}

fn parsed_absolute_episode_token(parsed_episode: &ParsedEpisodeMetadata) -> Option<String> {
    if !parsed_episode.absolute_episode_numbers.is_empty() {
        format_number_token(parsed_episode.absolute_episode_numbers.clone(), 3, true)
    } else {
        parsed_episode
            .absolute_episode
            .map(|value| format!("{value:03}"))
    }
}

fn episode_sort_key(
    episode: &Episode,
    collections_by_id: &HashMap<String, Collection>,
) -> (u32, u32, u32, u32, String) {
    let collection = episode
        .collection_id
        .as_deref()
        .and_then(|collection_id| collections_by_id.get(collection_id));

    (
        collection
            .and_then(|value| {
                parse_sort_number(
                    value
                        .narrative_order
                        .as_deref()
                        .or(Some(value.collection_index.as_str())),
                )
            })
            .unwrap_or(u32::MAX),
        parse_sort_number(episode.season_number.as_deref()).unwrap_or(u32::MAX),
        parse_sort_number(episode.episode_number.as_deref()).unwrap_or(u32::MAX),
        parse_sort_number(episode.absolute_number.as_deref()).unwrap_or(u32::MAX),
        episode.id.clone(),
    )
}

fn parse_sort_number(value: Option<&str>) -> Option<u32> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u32>().ok())
}

fn parse_interstitial_season_episode(value: Option<&str>) -> Option<(String, String)> {
    let raw = value?.trim();
    let stripped = raw.strip_prefix('S')?;
    let (season, episode) = stripped.split_once('E')?;
    let season = season.trim_start_matches('0');
    let episode = episode.trim_start_matches('0');
    Some((
        if season.is_empty() {
            "0".to_string()
        } else {
            season.to_string()
        },
        if episode.is_empty() {
            "0".to_string()
        } else {
            episode.to_string()
        },
    ))
}

fn non_empty_owned(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        if value.trim().is_empty() {
            None
        } else {
            Some(value)
        }
    })
}

fn normalize_absolute_episode_token(value: Option<String>) -> Option<String> {
    non_empty_owned(value).map(|value| match value.parse::<u32>() {
        Ok(number) => format!("{number:03}"),
        Err(_) => value,
    })
}

fn non_empty_string(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

pub(crate) fn build_rename_plan_from_items(
    facet: MediaFacet,
    title_id: Option<String>,
    template: String,
    collision_policy: RenameCollisionPolicy,
    missing_metadata_policy: RenameMissingMetadataPolicy,
    items: Vec<RenamePlanItem>,
) -> RenamePlan {
    let total = items.len();
    let renamable = items
        .iter()
        .filter(|item| matches!(item.write_action, RenameWriteAction::Move))
        .count();
    let noop = items
        .iter()
        .filter(|item| matches!(item.write_action, RenameWriteAction::Noop))
        .count();
    let conflicts = items.iter().filter(|item| item.collision).count();
    let errors = items
        .iter()
        .filter(|item| matches!(item.write_action, RenameWriteAction::Error))
        .count();

    let fingerprint = build_rename_plan_fingerprint(
        &items,
        &template,
        &collision_policy,
        &missing_metadata_policy,
    );

    RenamePlan {
        facet,
        title_id,
        template,
        collision_policy,
        missing_metadata_policy,
        fingerprint,
        total,
        renamable,
        noop,
        conflicts,
        errors,
        items,
    }
}

pub(crate) fn build_movie_rename_plan_item(
    title: &Title,
    collection: &Collection,
    media_file: Option<&TitleMediaFile>,
    template: &str,
    collision_policy: &RenameCollisionPolicy,
    missing_metadata_policy: &RenameMissingMetadataPolicy,
    planned_targets: &mut HashSet<String>,
) -> RenamePlanItem {
    let item_ids = RenamePlanItemIds {
        collection_id: Some(collection.id.clone()),
        media_file_id: media_file.map(|media_file| media_file.id.clone()),
    };
    let source_file =
        match prepare_rename_plan_source(item_ids.clone(), collection.ordered_path.clone()) {
            Ok(source_file) => source_file,
            Err(item) => return *item,
        };
    let current_stem = source_file
        .current_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let parsed = parse_release_metadata(current_stem);
    let (title_token, year_token) = split_title_and_year_hint(&title.name);
    let extension = source_file.extension.clone();
    let mut common = resolve_rename_common_metadata(
        media_file,
        &parsed,
        &title_token,
        year_token.as_deref(),
        &extension,
    );
    if common.common.quality.is_empty() {
        common.common.quality = collection
            .label
            .clone()
            .or(parsed.quality.clone())
            .unwrap_or_default();
    }

    let mut tokens = BTreeMap::new();
    let edition = common.edition.clone();
    insert_common_rename_tokens(&mut tokens, common.common);
    tokens.insert("edition".to_string(), edition);
    let rendered = match resolve_rendered_rename_filename(
        &source_file,
        item_ids.clone(),
        template,
        &tokens,
        &title_token,
        missing_metadata_policy,
    ) {
        Ok(rendered) => rendered,
        Err(item) => return *item,
    };

    finalize_rename_plan_item(
        &source_file,
        item_ids,
        rendered,
        collision_policy,
        planned_targets,
    )
}

fn split_title_and_year_hint(raw_title: &str) -> (String, Option<String>) {
    let trimmed = raw_title.trim();
    for (open, close) in [('(', ')'), ('[', ']')] {
        if let Some(close_pos) = trimmed.rfind(close)
            && let Some(open_pos) = trimmed[..close_pos].rfind(open)
        {
            let candidate = trimmed[open_pos + 1..close_pos].trim();
            if candidate.len() == 4 && candidate.chars().all(|value| value.is_ascii_digit()) {
                let title = trimmed[..open_pos].trim().to_string();
                if !title.is_empty() {
                    return (title, Some(candidate.to_string()));
                }
            }
        }
    }

    (trimmed.to_string(), None)
}

fn resolve_template_token(tokens: &BTreeMap<String, String>, token_spec: &str) -> String {
    let (name, pad_width) = match token_spec.split_once(':') {
        Some((n, fmt)) => (n.trim().to_lowercase(), fmt.trim().parse::<usize>().ok()),
        None => (token_spec.trim().to_lowercase(), None),
    };
    let raw = tokens.get(&name).cloned().unwrap_or_default();
    match pad_width {
        Some(width) if width > 0 => {
            if raw.chars().all(|c| c.is_ascii_digit()) && !raw.is_empty() {
                format!("{:0>width$}", raw, width = width)
            } else {
                raw
            }
        }
        _ => raw,
    }
}

pub fn sanitize_filesystem_component(raw: &str) -> String {
    let mut sanitized = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            sanitized.push(' ');
        } else {
            sanitized.push(ch);
        }
    }

    collapse_separators(&sanitized)
}

fn collapse_separators(raw: &str) -> String {
    let mut collapsed = String::with_capacity(raw.len());
    let mut previous: Option<char> = None;

    for ch in raw.chars() {
        let normalized = if ch.is_whitespace() { ' ' } else { ch };
        let is_separator = matches!(normalized, ' ' | '.' | '-' | '_');
        if is_separator && previous.is_some_and(|prev| prev == normalized) {
            continue;
        }
        collapsed.push(normalized);
        previous = Some(normalized);
    }

    collapsed
        .trim_matches(|value: char| value.is_whitespace() || matches!(value, '.' | '-' | '_'))
        .to_string()
}

#[cfg(test)]
#[path = "library_rename_tests.rs"]
mod library_rename_tests;
