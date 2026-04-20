use scryer_application::{AppError, AppResult, QualityProfile};
use scryer_domain::{BlocklistEntry, Episode, TitleHistoryRecord};
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc, oneshot};

use crate::commands::{DbCommand, spawn_db_command_worker};
use crate::encryption::EncryptionKey;
use crate::types::MigrationMode;

const DEFAULT_SQLITE_MAX_CONNECTIONS: u32 = 16;
const MAX_SQLITE_CONNECTIONS_CAP: u32 = 64;

fn sqlite_max_connections_from_env() -> u32 {
    std::env::var("SCRYER_SQLITE_MAX_CONNECTIONS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SQLITE_MAX_CONNECTIONS)
        .clamp(1, MAX_SQLITE_CONNECTIONS_CAP)
}

#[derive(Clone)]
pub struct DbRuntime {
    pub(crate) sender: mpsc::Sender<DbCommand>,
    pub(crate) pool: sqlx::SqlitePool,
    pub(crate) encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

pub type SqliteServices = DbRuntime;

impl DbRuntime {
    /// Public pool accessor for cross-crate query access.
    pub fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }

    pub async fn new(path: impl AsRef<str>) -> Result<Self, AppError> {
        Self::new_with_mode(path, MigrationMode::Apply).await
    }

    pub async fn new_with_mode(
        path: impl AsRef<str>,
        migration_mode: MigrationMode,
    ) -> Result<Self, AppError> {
        let db_url = crate::sqlite_url_with_create(path.as_ref());
        let is_memory = db_url.contains(":memory:");

        // Ensure the parent directory exists for file-based databases.
        if !is_memory
            && let Some(file_path) = path
                .as_ref()
                .strip_prefix("sqlite://")
                .or(Some(path.as_ref()))
        {
            let file_path = file_path.split('?').next().unwrap_or(file_path);
            let db_file = std::path::Path::new(file_path);
            if let Some(parent) = db_file.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    tracing::info!(path = %parent.display(), "creating database directory");
                    std::fs::create_dir_all(parent).map_err(|err| {
                        AppError::Repository(format!(
                            "cannot create database directory {}: {err}",
                            parent.display(),
                        ))
                    })?;
                }
                // Log diagnostic info for troubleshooting permission issues.
                if parent.exists() {
                    let meta = std::fs::metadata(parent);
                    let probe = parent.join(".scryer-probe");
                    let writable = std::fs::File::create(&probe).is_ok();
                    let _ = std::fs::remove_file(&probe);
                    tracing::debug!(
                        path = %parent.display(),
                        writable,
                        permissions = ?meta.as_ref().map(|m| m.permissions()),
                        "database directory check",
                    );
                }
            }
        }

        let pool_opts = if is_memory {
            // For in-memory databases the data only lives as long as at least one
            // connection is open.  Prevent the pool from recycling/dropping the
            // single connection (via idle_timeout or max_lifetime) which would
            // silently destroy every table.
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .min_connections(1)
                .idle_timeout(None)
                .max_lifetime(None)
        } else {
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(sqlite_max_connections_from_env())
        };

        // Build connect options so every connection gets WAL + busy_timeout.
        // sqlx already defaults foreign_keys = ON for SQLite.
        let mut connect_opts: sqlx::sqlite::SqliteConnectOptions = db_url
            .parse()
            .map_err(|err: sqlx::Error| AppError::Repository(err.to_string()))?;
        if !is_memory {
            connect_opts = connect_opts
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .busy_timeout(std::time::Duration::from_millis(5000));
        }

        let pool = pool_opts.connect_with(connect_opts).await.map_err(|err| {
            AppError::Repository(format!("cannot open database at {}: {err}", path.as_ref(),))
        })?;

        crate::migrations::run_migrations(&pool, migration_mode)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        let sender = spawn_db_command_worker(pool.clone());

        Ok(Self {
            sender,
            pool,
            encryption_key: Arc::new(RwLock::new(None)),
        })
    }

    pub(crate) fn encryption_key_state(&self) -> Arc<RwLock<Option<EncryptionKey>>> {
        self.encryption_key.clone()
    }

    pub async fn set_encryption_key(&self, key: crate::encryption::EncryptionKey) -> AppResult<()> {
        *self
            .encryption_key
            .write()
            .map_err(|_| AppError::Repository("encryption key lock poisoned".to_string()))? =
            Some(key);

        Ok(())
    }

    pub async fn upsert_library_scan_unmatched_item(
        &self,
        item: &scryer_application::LibraryScanUnmatchedItem,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpsertLibraryScanUnmatchedItem {
                item: item.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_library_scan_unmatched_item(
        &self,
        facet: scryer_domain::MediaFacet,
        item_path: &str,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteLibraryScanUnmatchedItem {
                facet,
                item_path: item_path.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_library_scan_unmatched_items(
        &self,
        facet: Option<scryer_domain::MediaFacet>,
        scan_root: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<scryer_application::LibraryScanUnmatchedItem>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListLibraryScanUnmatchedItems {
                facet,
                scan_root: scan_root.map(str::to_string),
                limit,
                offset,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn count_library_scan_unmatched_items(
        &self,
        facet: Option<scryer_domain::MediaFacet>,
        scan_root: Option<&str>,
    ) -> AppResult<i64> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CountLibraryScanUnmatchedItems {
                facet,
                scan_root: scan_root.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }
    pub async fn list_quality_profiles(
        &self,
        scope: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<Vec<QualityProfile>> {
        let scope = scope.into();
        crate::queries::quality::list_quality_profiles_query(&self.pool, &scope, scope_id).await
    }

    pub async fn replace_quality_profiles(
        &self,
        scope: impl Into<String>,
        scope_id: Option<String>,
        profiles: Vec<QualityProfile>,
    ) -> AppResult<()> {
        let scope = scope.into();
        crate::queries::quality::replace_quality_profiles_query(
            &self.pool, &scope, scope_id, profiles,
        )
        .await
    }

    pub async fn upsert_quality_profiles(
        &self,
        scope: impl Into<String>,
        scope_id: Option<String>,
        profiles: Vec<QualityProfile>,
    ) -> AppResult<()> {
        let scope = scope.into();
        crate::queries::quality::upsert_quality_profiles_query(
            &self.pool, &scope, scope_id, profiles,
        )
        .await
    }

    pub async fn delete_quality_profile(&self, profile_id: impl Into<String>) -> AppResult<()> {
        crate::queries::quality::delete_quality_profile_query(&self.pool, &profile_id.into()).await
    }

    pub async fn insert_media_file(
        &self,
        input: &scryer_application::InsertMediaFileInput,
    ) -> AppResult<String> {
        crate::queries::media_file::insert_media_file_query(&self.pool, input).await
    }

    pub async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        crate::queries::media_file::link_file_to_episode_query(&self.pool, file_id, episode_id)
            .await
    }

    pub async fn list_media_files_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_application::TitleMediaFile>> {
        crate::queries::media_file::list_media_files_for_title_query(&self.pool, title_id).await
    }

    pub async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<scryer_application::TitleMediaSizeSummary>> {
        crate::queries::media_file::list_title_media_size_summaries_query(&self.pool, title_ids)
            .await
    }

    pub async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<scryer_application::TitleEpisodeProgressSummary>> {
        crate::queries::media_file::list_title_episode_progress_summaries_query(
            &self.pool, title_ids,
        )
        .await
    }

    pub async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: scryer_application::MediaFileAnalysis,
    ) -> AppResult<()> {
        crate::queries::media_file::update_media_file_analysis_query(&self.pool, file_id, &analysis)
            .await
    }

    pub async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        crate::queries::media_file::update_media_file_source_signature_query(
            &self.pool,
            file_id,
            size_bytes,
            source_signature_scheme.as_deref(),
            source_signature_value.as_deref(),
        )
        .await
    }

    pub async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        crate::queries::media_file::update_media_file_path_query(&self.pool, file_id, file_path)
            .await
    }

    pub async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        crate::queries::media_file::mark_scan_failed_query(&self.pool, file_id, error).await
    }

    pub async fn get_media_file_by_id(
        &self,
        file_id: &str,
    ) -> AppResult<Option<scryer_application::TitleMediaFile>> {
        crate::queries::media_file::get_media_file_by_id_query(&self.pool, file_id).await
    }

    pub async fn get_media_file_by_path(
        &self,
        file_path: &str,
    ) -> AppResult<Option<scryer_application::TitleMediaFile>> {
        crate::queries::media_file::get_media_file_by_path_query(&self.pool, file_path).await
    }

    pub async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        crate::queries::media_file::delete_media_file_query(&self.pool, file_id).await
    }

    pub async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListEpisodesForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::FindEpisodeByTitleAndNumbers {
                title_id: title_id.to_string(),
                season_number: season_number.to_string(),
                episode_number: episode_number.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::FindEpisodeByTitleAndAbsoluteNumber {
                title_id: title_id.to_string(),
                absolute_number: absolute_number.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn upsert_wanted_item(
        &self,
        item: &scryer_application::WantedItem,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpsertWantedItem {
                item: item.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn ensure_wanted_item_seeded_atomic(
        &self,
        item: scryer_application::WantedItem,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::EnsureWantedItemSeeded {
                item,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_due_wanted_items(
        &self,
        now: &str,
        batch_limit: i64,
    ) -> AppResult<Vec<scryer_application::WantedItem>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListDueWantedItems {
                now: now.to_string(),
                batch_limit,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    #[expect(clippy::too_many_arguments)]
    pub async fn update_wanted_item_status(
        &self,
        id: &str,
        status: &str,
        next_search_at: Option<&str>,
        last_search_at: Option<&str>,
        search_count: i64,
        current_score: Option<i32>,
        grabbed_release: Option<&str>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateWantedItemStatus {
                id: id.to_string(),
                status: status.to_string(),
                next_search_at: next_search_at.map(str::to_string),
                last_search_at: last_search_at.map(str::to_string),
                search_count,
                current_score,
                grabbed_release: grabbed_release.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn get_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<scryer_application::WantedItem>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::GetWantedItemForTitle {
                title_id: title_id.to_string(),
                episode_id: episode_id.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn reset_fruitless_wanted_items(&self, now: &str) -> AppResult<u64> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ResetFruitlessWantedItems {
                now: now.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteWantedItemsForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_wanted_items_for_collection(&self, collection_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteWantedItemsForCollection {
                collection_id: collection_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_wanted_items_for_episode(&self, episode_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteWantedItemsForEpisode {
                episode_id: episode_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn insert_release_decision(
        &self,
        decision: &scryer_application::ReleaseDecision,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertReleaseDecision {
                decision: decision.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn get_wanted_item_by_id(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_application::WantedItem>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::GetWantedItemById {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_wanted_items(
        &self,
        status: Option<&str>,
        media_type: Option<&str>,
        title_id: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<scryer_application::WantedItem>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListWantedItems {
                status: status.map(str::to_string),
                media_type: media_type.map(str::to_string),
                title_id: title_id.map(str::to_string),
                limit,
                offset,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn count_wanted_items(
        &self,
        status: Option<&str>,
        media_type: Option<&str>,
        title_id: Option<&str>,
    ) -> AppResult<i64> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CountWantedItems {
                status: status.map(str::to_string),
                media_type: media_type.map(str::to_string),
                title_id: title_id.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<scryer_application::ReleaseDecision>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListReleaseDecisionsForTitle {
                title_id: title_id.to_string(),
                limit,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_release_decisions_for_wanted_item(
        &self,
        wanted_item_id: &str,
        limit: i64,
    ) -> AppResult<Vec<scryer_application::ReleaseDecision>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListReleaseDecisionsForWantedItem {
                wanted_item_id: wanted_item_id.to_string(),
                limit,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn insert_pending_release(
        &self,
        release: &scryer_application::PendingRelease,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertPendingRelease {
                release: release.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_expired_pending_releases(
        &self,
        now: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListExpiredPendingReleases {
                now: now.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListPendingReleasesForWantedItem {
                wanted_item_id: wanted_item_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_pending_release_status(
        &self,
        id: &str,
        status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdatePendingReleaseStatus {
                id: id.to_string(),
                status,
                grabbed_at: grabbed_at.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListStandbyPendingReleasesForWantedItem {
                wanted_item_id: wanted_item_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteStandbyPendingReleasesForWantedItem {
                wanted_item_id: wanted_item_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_all_standby_pending_releases(
        &self,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListAllStandbyPendingReleases { reply: reply_tx })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: scryer_application::PendingReleaseStatus,
        next_status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CompareAndSetPendingReleaseStatus {
                id: id.to_string(),
                current_status,
                next_status,
                grabbed_at: grabbed_at.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_waiting_pending_releases(
        &self,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListWaitingPendingReleases { reply: reply_tx })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn get_pending_release(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_application::PendingRelease>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::GetPendingRelease {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn supersede_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::SupersedePendingReleasesForWantedItem {
                wanted_item_id: wanted_item_id.to_string(),
                except_id: except_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeletePendingReleasesForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    // ── Title History ─────────────────────────────────────────────────────────

    #[expect(clippy::too_many_arguments)]
    pub async fn insert_title_history_event(
        &self,
        title_id: String,
        episode_id: Option<String>,
        collection_id: Option<String>,
        event_type: String,
        source_title: Option<String>,
        quality: Option<String>,
        download_id: Option<String>,
        data_json: Option<String>,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertTitleHistoryEvent {
                title_id,
                episode_id,
                collection_id,
                event_type,
                source_title,
                quality,
                download_id,
                data_json,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_title_history(
        &self,
        event_types: Option<Vec<String>>,
        title_ids: Option<Vec<String>>,
        download_id: Option<String>,
        limit: usize,
        offset: usize,
    ) -> AppResult<(Vec<TitleHistoryRecord>, i64)> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListTitleHistory {
                event_types,
                title_ids,
                download_id,
                limit,
                offset,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_title_history_for_title(
        &self,
        title_id: &str,
        event_types: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> AppResult<(Vec<TitleHistoryRecord>, i64)> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListTitleHistoryForTitle {
                title_id: title_id.to_string(),
                event_types,
                limit,
                offset,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_title_history_for_episode(
        &self,
        episode_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleHistoryRecord>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListTitleHistoryForEpisode {
                episode_id: episode_id.to_string(),
                limit,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn find_title_history_by_download_id(
        &self,
        download_id: &str,
    ) -> AppResult<Vec<TitleHistoryRecord>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::FindTitleHistoryByDownloadId {
                download_id: download_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_title_history_for_title(&self, title_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteTitleHistoryForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    // ── Blocklist ─────────────────────────────────────────────────────────────

    #[expect(clippy::too_many_arguments)]
    pub async fn insert_blocklist_entry(
        &self,
        title_id: String,
        source_title: Option<String>,
        source_hint: Option<String>,
        quality: Option<String>,
        download_id: Option<String>,
        reason: Option<String>,
        data_json: Option<String>,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertBlocklistEntry {
                title_id,
                source_title,
                source_hint,
                quality,
                download_id,
                reason,
                data_json,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_blocklist_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<BlocklistEntry>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListBlocklistForTitle {
                title_id: title_id.to_string(),
                limit,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn list_blocklist_all(
        &self,
        limit: usize,
        offset: usize,
    ) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListBlocklistAll {
                limit,
                offset,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_blocklist_entry(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteBlocklistEntry {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn is_blocklisted(&self, title_id: &str, source_title: &str) -> AppResult<bool> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::IsBlocklisted {
                title_id: title_id.to_string(),
                source_title: source_title.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_blocklist_for_title(&self, title_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteBlocklistForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }
}
