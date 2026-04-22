use scryer_application::{
    AppError, AppResult, CollectionUpdate, CreateTitleOutcome, DownloadClientConfigUpdate,
    DownloadQueueCommandRecord, EpisodeUpdate, ExternalImportMonitorSnapshot, ImportArtifact,
    IndexerConfigUpdate, QualityProfile, ReleaseDownloadAttemptOutcome, SuccessfulGrabCommit,
    TitleImageReplacement, TitleMetadataUpdate, WorkflowOperationInfo,
};
use scryer_domain::{
    BlocklistEntry, Collection, DomainEvent, DownloadClientConfig, DownloadQueueDeleteStatus,
    Episode, ExternalId, ImportStatus, ImportType, IndexerConfig, InterstitialMovieMetadata,
    MediaFacet, NewDomainEvent, NotificationChannelConfig, NotificationSubscription,
    PluginInstallation, PostProcessingScript, PostProcessingScriptRun, RuleSet, SubtitleDownload,
    Title, User,
};
use sqlx::ConnectOptions;
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc, oneshot};

use crate::commands::{DbCommand, spawn_db_command_worker};
use crate::encryption::EncryptionKey;
use crate::types::MigrationMode;
use crate::types::{SettingDefinitionSeed, SettingsValueRecord};

const DEFAULT_SQLITE_MAX_CONNECTIONS: u32 = 16;
const MAX_SQLITE_CONNECTIONS_CAP: u32 = 64;
const SQLITE_SLOW_STATEMENT_WARN_MS: u64 = 500;

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
        // The serialized worker hides the hottest scan writes, but direct
        // pool-backed writes still rely on SQLite's built-in wait before they
        // bubble up as repository errors.
        // sqlx already defaults foreign_keys = ON for SQLite.
        let mut connect_opts: sqlx::sqlite::SqliteConnectOptions = db_url
            .parse()
            .map_err(|err: sqlx::Error| AppError::Repository(err.to_string()))?;
        connect_opts = connect_opts.log_slow_statements(
            tracing::log::LevelFilter::Warn,
            std::time::Duration::from_millis(SQLITE_SLOW_STATEMENT_WARN_MS),
        );
        if !is_memory {
            connect_opts = connect_opts
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .busy_timeout(std::time::Duration::from_millis(10_000));
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

    fn current_encryption_key(&self) -> AppResult<Option<EncryptionKey>> {
        self.encryption_key
            .read()
            .map(|value| value.clone())
            .map_err(|_| AppError::Repository("encryption key lock poisoned".to_string()))
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
        crate::queries::library_scan_unmatched::list_library_scan_unmatched_items_query(
            &self.pool, facet, scan_root, limit, offset,
        )
        .await
    }

    pub async fn count_library_scan_unmatched_items(
        &self,
        facet: Option<scryer_domain::MediaFacet>,
        scan_root: Option<&str>,
    ) -> AppResult<i64> {
        crate::queries::library_scan_unmatched::count_library_scan_unmatched_items_query(
            &self.pool, facet, scan_root,
        )
        .await
    }

    pub async fn create_or_get_existing_title(
        &self,
        title: &Title,
    ) -> AppResult<CreateTitleOutcome> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateOrGetExistingTitle {
                title: title.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn replace_title_image(
        &self,
        title_id: &str,
        replacement: TitleImageReplacement,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ReplaceTitleImage {
                title_id: title_id.to_string(),
                replacement,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn set_title_folder_path(&self, title_id: &str, folder_path: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::SetTitleFolderPath {
                title_id: title_id.to_string(),
                folder_path: folder_path.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_collection(&self, collection: &Collection) -> AppResult<Collection> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateCollection {
                collection: collection.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn append_domain_event(&self, event: &NewDomainEvent) -> AppResult<DomainEvent> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::AppendDomainEvent {
                event: event.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn set_event_subscriber_offset(
        &self,
        subscriber: &str,
        sequence: i64,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::SetEventSubscriberOffset {
                subscriber: subscriber.to_string(),
                sequence,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_title_metadata(
        &self,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags_json: Option<String>,
    ) -> AppResult<Title> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateTitleMetadata {
                id: id.to_string(),
                name,
                facet,
                tags_json,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateTitleHydratedMetadata {
                id: id.to_string(),
                metadata,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn replace_title_match_state(
        &self,
        id: &str,
        external_ids: Vec<ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ReplaceTitleMatchState {
                id: id.to_string(),
                external_ids,
                tags,
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
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ReplaceQualityProfiles {
                scope,
                scope_id,
                profiles,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn upsert_quality_profiles(
        &self,
        scope: impl Into<String>,
        scope_id: Option<String>,
        profiles: Vec<QualityProfile>,
    ) -> AppResult<()> {
        let scope = scope.into();
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpsertQualityProfiles {
                scope,
                scope_id,
                profiles,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_quality_profile(&self, profile_id: impl Into<String>) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteQualityProfile {
                profile_id: profile_id.into(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn insert_media_file(
        &self,
        input: &scryer_application::InsertMediaFileInput,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertMediaFile {
                input: input.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::LinkFileToEpisode {
                file_id: file_id.to_string(),
                episode_id: episode_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
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

    pub async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<scryer_application::TitleQualitySummary>> {
        crate::queries::media_file::list_title_quality_summaries_query(&self.pool, title_ids).await
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
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateMediaFileAnalysis {
                file_id: file_id.to_string(),
                analysis,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateMediaFileSourceSignature {
                file_id: file_id.to_string(),
                size_bytes,
                source_signature_scheme,
                source_signature_value,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateMediaFilePath {
                file_id: file_id.to_string(),
                file_path: file_path.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::MarkScanFailed {
                file_id: file_id.to_string(),
                error: error.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
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
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteMediaFile {
                file_id: file_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
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

    pub async fn create_episode(&self, episode: &Episode) -> AppResult<Episode> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateEpisode {
                episode: episode.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_episode(
        &self,
        episode_id: &str,
        update: EpisodeUpdate,
    ) -> AppResult<Episode> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateEpisode {
                episode_id: episode_id.to_string(),
                update,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteEpisode {
                episode_id: episode_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn recover_stale_running_delete_download_commands(
        &self,
        stale_seconds: i64,
    ) -> AppResult<u64> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::RecoverStaleRunningDeleteDownloadCommands {
                stale_seconds,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_delete_download_command_status(
        &self,
        id: &str,
        status: DownloadQueueDeleteStatus,
        error_text: Option<&str>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateDeleteDownloadCommandStatus {
                id: id.to_string(),
                status,
                error_text: error_text.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_title(&self, title: &Title) -> AppResult<Title> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateTitle {
                title: title.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn mark_title_metadata_hydration_due_now(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::MarkTitleMetadataHydrationDueNow {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn schedule_title_metadata_hydration_retry(
        &self,
        id: &str,
        next_attempt_at: &str,
        attempt_count: i64,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ScheduleTitleMetadataHydrationRetry {
                id: id.to_string(),
                next_attempt_at: next_attempt_at.to_string(),
                attempt_count,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn clear_title_metadata_hydration_retry_state(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ClearTitleMetadataHydrationRetryState {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_title_monitored(&self, id: &str, monitored: bool) -> AppResult<Title> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateTitleMonitored {
                id: id.to_string(),
                monitored,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_title(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteTitle {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn clear_title_folder_path(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ClearTitleFolderPath {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ClearMetadataLanguageForAll { reply: reply_tx })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_collection(
        &self,
        collection_id: &str,
        update: CollectionUpdate,
    ) -> AppResult<Collection> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateCollection {
                collection_id: collection_id.to_string(),
                update,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_collection_interstitial_movie(
        &self,
        collection_id: &str,
        interstitial_movie: &InterstitialMovieMetadata,
    ) -> AppResult<Collection> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateCollectionInterstitialMovie {
                collection_id: collection_id.to_string(),
                interstitial_movie: interstitial_movie.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_collection_specials_movies(
        &self,
        collection_id: &str,
        specials_movies: &[InterstitialMovieMetadata],
    ) -> AppResult<Collection> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateCollectionSpecialsMovies {
                collection_id: collection_id.to_string(),
                specials_movies: specials_movies.to_vec(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_interstitial_season_episode(
        &self,
        collection_id: &str,
        season_episode: Option<&str>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateInterstitialSeasonEpisode {
                collection_id: collection_id.to_string(),
                season_episode: season_episode.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::SetCollectionEpisodesMonitored {
                collection_id: collection_id.to_string(),
                monitored,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteCollection {
                collection_id: collection_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteCollectionsForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteEpisodesForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_user(&self, user: &User) -> AppResult<User> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateUser {
                user: user.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_user_entitlements(
        &self,
        id: &str,
        entitlements_json: &str,
    ) -> AppResult<User> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateUserEntitlements {
                id: id.to_string(),
                entitlements_json: entitlements_json.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_user_password_hash(
        &self,
        id: &str,
        password_hash: &str,
    ) -> AppResult<User> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateUserPasswordHash {
                id: id.to_string(),
                password_hash: password_hash.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_user(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteUser {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CommitSuccessfulGrab {
                commit: commit.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn append_domain_events(
        &self,
        events: Vec<NewDomainEvent>,
    ) -> AppResult<Vec<DomainEvent>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::AppendDomainEvents {
                events,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn record_download_submission(
        &self,
        submission: &scryer_application::DownloadSubmission,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::RecordDownloadSubmission {
                submission: submission.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_download_submissions_for_title(&self, title_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteDownloadSubmissionsForTitle {
                title_id: title_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_download_submission_by_client_item_id(
        &self,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteDownloadSubmissionByClientItemId {
                download_client_item_id: download_client_item_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_tracked_state(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
        tracked_state: &str,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateTrackedState {
                download_client_type: download_client_type.to_string(),
                download_client_item_id: download_client_item_id.to_string(),
                tracked_state: tracked_state.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn insert_import_artifact(&self, artifact: &ImportArtifact) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertImportArtifact {
                artifact: artifact.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    #[expect(clippy::too_many_arguments)]
    pub async fn create_job_workflow_operation(
        &self,
        operation_type: String,
        status: String,
        job_key: String,
        trigger_source: String,
        actor_user_id: Option<String>,
        progress_json: Option<String>,
        summary_json: Option<String>,
        summary_text: Option<String>,
        error_text: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
    ) -> AppResult<crate::WorkflowOperationRecord> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateJobWorkflowOperation {
                operation_type,
                status,
                job_key,
                trigger_source,
                actor_user_id,
                progress_json,
                summary_json,
                summary_text,
                error_text,
                started_at,
                completed_at,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    #[expect(clippy::too_many_arguments)]
    pub async fn update_job_workflow_operation(
        &self,
        id: &str,
        status: &str,
        progress_json: Option<String>,
        summary_json: Option<String>,
        summary_text: Option<String>,
        error_text: Option<String>,
        completed_at: Option<String>,
    ) -> AppResult<crate::WorkflowOperationRecord> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateJobWorkflowOperation {
                id: id.to_string(),
                status: status.to_string(),
                progress_json,
                summary_json,
                summary_text,
                error_text,
                completed_at,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_import_request(
        &self,
        source_system: String,
        source_ref: String,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateImportRequest {
                source_system,
                source_ref,
                import_type,
                payload_json,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_import_status(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateImportStatus {
                import_id: import_id.to_string(),
                status: status.as_str().to_string(),
                result_json,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn recover_stale_processing_imports(&self, stale_seconds: i64) -> AppResult<u64> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::RecoverStaleProcessingImports {
                stale_seconds,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn recover_stale_processing_imports_for_type(
        &self,
        import_type: ImportType,
        stale_seconds: i64,
    ) -> AppResult<u64> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::RecoverStaleProcessingImportsForType {
                import_type,
                stale_seconds,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn upsert_external_import_monitor_snapshot(
        &self,
        snapshot: &ExternalImportMonitorSnapshot,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpsertExternalImportMonitorSnapshot {
                snapshot: snapshot.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_external_import_monitor_snapshot(
        &self,
        facet: MediaFacet,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteExternalImportMonitorSnapshot {
                facet,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn queue_delete_download_command(
        &self,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<DownloadQueueCommandRecord> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::QueueDeleteDownloadCommand {
                client_type: client_type.to_string(),
                download_client_item_id: download_client_item_id.to_string(),
                is_history,
                requested_by_user_id: requested_by_user_id.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn prune_terminal_delete_download_commands_older_than(
        &self,
        days: i64,
    ) -> AppResult<u32> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::PruneTerminalDeleteDownloadCommandsOlderThan {
                days,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_workflow_operation(
        &self,
        operation_type: String,
        status: String,
        actor_user_id: Option<String>,
        progress_json: Option<String>,
        started_at: Option<String>,
        completed_at: Option<String>,
    ) -> AppResult<WorkflowOperationInfo> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateWorkflowOperation {
                operation_type,
                status,
                actor_user_id,
                progress_json,
                started_at,
                completed_at,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn upsert_library_probe_signature(
        &self,
        title_id: &str,
        path: &str,
        probe_signature_scheme: Option<String>,
        probe_signature_value: Option<String>,
        last_probed_at: Option<String>,
        last_changed_at: Option<String>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpsertLibraryProbeSignature {
                title_id: title_id.to_string(),
                path: path.to_string(),
                probe_signature_scheme,
                probe_signature_value,
                last_probed_at,
                last_changed_at,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn complete_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
        last_search_at: Option<&str>,
        current_score: Option<i32>,
    ) -> AppResult<bool> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CompleteWantedItemForTitle {
                title_id: title_id.to_string(),
                episode_id: episode_id.map(str::to_string),
                last_search_at: last_search_at.map(str::to_string),
                current_score,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteReleaseDecisionsOlderThan {
                days,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteReleaseAttemptsOlderThan {
                days,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_dispatched_event_outboxes_older_than(&self, days: i64) -> AppResult<u32> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteDispatchedEventOutboxesOlderThan {
                days,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteHistoryEventsOlderThan {
                days,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_domain_events_older_than_for_types(
        &self,
        days: i64,
        event_types: &[scryer_domain::DomainEventType],
    ) -> AppResult<u32> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteDomainEventsOlderThanForTypes {
                days,
                event_types: event_types.to_vec(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_download_import_artifacts_older_than(&self, days: i64) -> AppResult<u32> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteDownloadImportArtifactsOlderThan {
                days,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_terminal_imports_older_than(&self, days: i64) -> AppResult<u32> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteTerminalImportsOlderThan {
                days,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_terminal_download_queue_commands_older_than(
        &self,
        days: i64,
    ) -> AppResult<u32> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteTerminalDownloadQueueCommandsOlderThan {
                days,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_rule_set_history_older_than(&self, days: i64) -> AppResult<u32> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteRuleSetHistoryOlderThan {
                days,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteMediaFilesByIds {
                ids: ids.to_vec(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn insert_subtitle_download(&self, download: &SubtitleDownload) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::InsertSubtitleDownload {
                download: download.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn set_subtitle_download_synced(&self, id: &str, synced: bool) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::SetSubtitleDownloadSynced {
                id: id.to_string(),
                synced,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_subtitle_download(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteSubtitleDownload {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn blacklist_subtitle_download(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::BlacklistSubtitleDownload {
                media_file_id: media_file_id.to_string(),
                provider: provider.to_string(),
                provider_file_id: provider_file_id.to_string(),
                language: language.to_string(),
                reason: reason.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_indexer_config(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
        let encryption_key = self.current_encryption_key()?;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateIndexerConfig {
                config,
                encryption_key,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn touch_indexer_last_error(&self, provider_type: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::TouchIndexerLastError {
                provider_type: provider_type.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_indexer_config(
        &self,
        update: IndexerConfigUpdate,
    ) -> AppResult<IndexerConfig> {
        let encryption_key = self.current_encryption_key()?;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateIndexerConfig {
                update,
                encryption_key,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_indexer_config(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteIndexerConfig {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_download_client_config(
        &self,
        config: DownloadClientConfig,
    ) -> AppResult<DownloadClientConfig> {
        let encryption_key = self.current_encryption_key()?;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateDownloadClientConfig {
                config,
                encryption_key,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_download_client_config(
        &self,
        update: DownloadClientConfigUpdate,
    ) -> AppResult<DownloadClientConfig> {
        let encryption_key = self.current_encryption_key()?;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateDownloadClientConfig {
                update,
                encryption_key,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_download_client_config(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteDownloadClientConfig {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn reorder_download_client_configs(&self, ordered_ids: Vec<String>) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ReorderDownloadClientConfigs {
                ordered_ids,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn batch_ensure_setting_definitions(
        &self,
        definitions: Vec<SettingDefinitionSeed>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::BatchEnsureSettingDefinitions {
                definitions,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn batch_upsert_settings_if_not_overridden(
        &self,
        entries: Vec<(String, String, String, String)>,
    ) -> AppResult<()> {
        let encryption_key = self.current_encryption_key()?;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::BatchUpsertSettingsIfNotOverridden {
                entries,
                encryption_key,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn upsert_setting_value(
        &self,
        scope: impl Into<String>,
        key_name: impl Into<String>,
        scope_id: Option<String>,
        value_json: impl Into<String>,
        source: impl Into<String>,
        updated_by_user_id: Option<String>,
    ) -> AppResult<SettingsValueRecord> {
        let encryption_key = self.current_encryption_key()?;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpsertSettingValue {
                scope: scope.into(),
                key_name: key_name.into(),
                scope_id,
                value_json: value_json.into(),
                source: source.into(),
                updated_by_user_id,
                encryption_key,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_setting_value(
        &self,
        scope: impl Into<String>,
        key_name: impl Into<String>,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteSettingValue {
                scope: scope.into(),
                key_name: key_name.into(),
                scope_id,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn vacuum_into(&self, dest_path: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::VacuumInto {
                dest_path: dest_path.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateRuleSet {
                rule_set: rule_set.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_rule_set(&self, rule_set: &RuleSet) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateRuleSet {
                rule_set: rule_set.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteRuleSet {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn record_rule_set_history(
        &self,
        id: &str,
        rule_set_id: &str,
        action: &str,
        rego_source: Option<&str>,
        actor_id: Option<&str>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::RecordRuleSetHistory {
                id: id.to_string(),
                rule_set_id: rule_set_id.to_string(),
                action: action.to_string(),
                rego_source: rego_source.map(str::to_string),
                actor_id: actor_id.map(str::to_string),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteRuleSetByManagedKey {
                key: key.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_post_processing_script(
        &self,
        script: PostProcessingScript,
    ) -> AppResult<PostProcessingScript> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreatePostProcessingScript {
                script,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_post_processing_script(
        &self,
        script: PostProcessingScript,
    ) -> AppResult<PostProcessingScript> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdatePostProcessingScript {
                script,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_post_processing_script(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeletePostProcessingScript {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn record_post_processing_script_run(
        &self,
        run: PostProcessingScriptRun,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::RecordPostProcessingScriptRun {
                run,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreatePluginInstallation {
                installation: installation.clone(),
                wasm_bytes: wasm_bytes.map(|bytes| bytes.to_vec()),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdatePluginInstallation {
                installation: installation.clone(),
                wasm_bytes: wasm_bytes.map(|bytes| bytes.to_vec()),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeletePluginInstallation {
                plugin_id: plugin_id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn seed_builtin_plugin(
        &self,
        plugin_id: &str,
        name: &str,
        description: &str,
        version: &str,
        provider_type: &str,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::SeedBuiltinPlugin {
                plugin_id: plugin_id.to_string(),
                name: name.to_string(),
                description: description.to_string(),
                version: version.to_string(),
                provider_type: provider_type.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn store_plugin_registry_cache(&self, json: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::StorePluginRegistryCache {
                json: json.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_notification_channel(
        &self,
        config: NotificationChannelConfig,
    ) -> AppResult<NotificationChannelConfig> {
        let encryption_key = self.current_encryption_key()?;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateNotificationChannel {
                config,
                encryption_key,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_notification_channel(
        &self,
        config: NotificationChannelConfig,
    ) -> AppResult<NotificationChannelConfig> {
        let encryption_key = self.current_encryption_key()?;
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateNotificationChannel {
                config,
                encryption_key,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_notification_channel(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteNotificationChannel {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_notification_subscription(
        &self,
        subscription: NotificationSubscription,
    ) -> AppResult<NotificationSubscription> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateNotificationSubscription {
                subscription,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn update_notification_subscription(
        &self,
        subscription: NotificationSubscription,
    ) -> AppResult<NotificationSubscription> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::UpdateNotificationSubscription {
                subscription,
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn delete_notification_subscription(&self, id: &str) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::DeleteNotificationSubscription {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        reply_rx
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?
    }

    pub async fn create_release_download_attempt(
        &self,
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        outcome: ReleaseDownloadAttemptOutcome,
        error_message: Option<String>,
        source_password: Option<String>,
    ) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CreateReleaseDownloadAttempt {
                title_id,
                source_hint,
                source_title,
                outcome,
                error_message,
                source_password,
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
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<scryer_application::WantedItem>> {
        crate::queries::wanted::list_due_wanted_items_query(
            &self.pool,
            now,
            batch_limit,
            excluded_facets,
        )
        .await
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
        latest_decision_code: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<scryer_application::WantedItem>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::ListWantedItems {
                status: status.map(str::to_string),
                media_type: media_type.map(str::to_string),
                title_id: title_id.map(str::to_string),
                latest_decision_code: latest_decision_code.map(str::to_string),
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
        latest_decision_code: Option<&str>,
    ) -> AppResult<i64> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(DbCommand::CountWantedItems {
                status: status.map(str::to_string),
                media_type: media_type.map(str::to_string),
                title_id: title_id.map(str::to_string),
                latest_decision_code: latest_decision_code.map(str::to_string),
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
