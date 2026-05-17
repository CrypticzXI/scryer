use std::sync::{Arc, RwLock};

use scryer_application::{
    AppError, AppResult, BlocklistRepository, NewBlocklistEntry, PendingRelease,
    PendingReleaseRepository, PendingReleaseStatus, ReleaseDecision, WantedItem,
    WantedItemRepository, WantedItemsQuery,
};
use scryer_domain::BlocklistEntry;
use sqlx::ConnectOptions;

use crate::encryption::EncryptionKey;
use crate::storage::sql::runtime::{repo_err, run_with_sqlite_busy_retries};
use crate::storage::sqlite::writer::{SqliteWriterGate, new_writer_gate};
use crate::types::MigrationMode;
use crate::{BlocklistStore, PendingReleaseStore, WantedStore};

const DEFAULT_SQLITE_MAX_CONNECTIONS: u32 = 16;
const MAX_SQLITE_CONNECTIONS_CAP: u32 = 64;
const SQLITE_SLOW_STATEMENT_WARN_MS: u64 = 1000;

fn sqlite_max_connections_from_env() -> u32 {
    std::env::var("SCRYER_SQLITE_MAX_CONNECTIONS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SQLITE_MAX_CONNECTIONS)
        .clamp(1, MAX_SQLITE_CONNECTIONS_CAP)
}

#[derive(Clone)]
pub struct SqliteServices {
    pub(crate) pool: sqlx::SqlitePool,
    pub(crate) encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
    pub(crate) writer_gate: SqliteWriterGate,
}

impl SqliteServices {
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
        crate::spellfix::register_spellfix_auto_extension()?;

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
            // connection is open. Prevent pool recycling from destroying the DB.
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .min_connections(1)
                .idle_timeout(None)
                .max_lifetime(None)
        } else {
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(sqlite_max_connections_from_env())
        };

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
        if matches!(migration_mode, MigrationMode::Apply) {
            crate::queries::title_search::seed_title_search_projection_if_empty(&pool).await?;
        }

        Ok(Self {
            pool,
            encryption_key: Arc::new(RwLock::new(None)),
            writer_gate: new_writer_gate(),
        })
    }

    pub(crate) fn encryption_key_state(&self) -> Arc<RwLock<Option<EncryptionKey>>> {
        self.encryption_key.clone()
    }

    pub(crate) fn writer_gate(&self) -> SqliteWriterGate {
        self.writer_gate.clone()
    }

    pub async fn set_encryption_key(&self, key: crate::encryption::EncryptionKey) -> AppResult<()> {
        *self
            .encryption_key
            .write()
            .map_err(|_| AppError::Repository("encryption key lock poisoned".to_string()))? =
            Some(key);

        Ok(())
    }

    pub async fn vacuum_into(&self, dest_path: &str) -> AppResult<()> {
        let _guard = self.writer_gate.lock().await;
        let pool = self.pool.clone();
        let dest_path = dest_path.to_string();

        run_with_sqlite_busy_retries("vacuum_into", || {
            let pool = pool.clone();
            let dest_path = dest_path.clone();

            async move {
                sqlx::query("VACUUM INTO ?")
                    .bind(dest_path)
                    .execute(&pool)
                    .await
                    .map_err(repo_err)?;
                Ok(())
            }
        })
        .await
    }

    pub async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String> {
        let store = WantedStore::from_sqlite_services(self);
        WantedItemRepository::insert_release_decision(&store, decision).await
    }

    pub async fn get_wanted_item_by_id(&self, id: &str) -> AppResult<Option<WantedItem>> {
        let store = WantedStore::from_sqlite_services(self);
        WantedItemRepository::get_wanted_item_by_id(&store, id).await
    }

    pub async fn list_wanted_items(&self, query: WantedItemsQuery) -> AppResult<Vec<WantedItem>> {
        let store = WantedStore::from_sqlite_services(self);
        WantedItemRepository::list_wanted_items(&store, query).await
    }

    pub async fn count_wanted_items(&self, query: WantedItemsQuery) -> AppResult<i64> {
        let store = WantedStore::from_sqlite_services(self);
        WantedItemRepository::count_wanted_items(&store, query).await
    }

    pub async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        let store = WantedStore::from_sqlite_services(self);
        WantedItemRepository::list_release_decisions_for_title(&store, title_id, limit).await
    }

    pub async fn list_release_decisions_for_wanted_item(
        &self,
        wanted_item_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        let store = WantedStore::from_sqlite_services(self);
        WantedItemRepository::list_release_decisions_for_wanted_item(&store, wanted_item_id, limit)
            .await
    }

    pub async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String> {
        let store = PendingReleaseStore::from_sqlite_services(self);
        PendingReleaseRepository::insert_pending_release(&store, release).await
    }

    pub async fn list_expired_pending_releases(&self, now: &str) -> AppResult<Vec<PendingRelease>> {
        let store = PendingReleaseStore::from_sqlite_services(self);
        PendingReleaseRepository::list_expired_pending_releases(&store, now).await
    }

    pub async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        let store = PendingReleaseStore::from_sqlite_services(self);
        PendingReleaseRepository::list_waiting_pending_releases(&store).await
    }

    pub async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>> {
        let store = PendingReleaseStore::from_sqlite_services(self);
        PendingReleaseRepository::get_pending_release(&store, id).await
    }

    pub async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        let store = PendingReleaseStore::from_sqlite_services(self);
        PendingReleaseRepository::list_pending_releases_for_wanted_item(&store, wanted_item_id)
            .await
    }

    pub async fn update_pending_release_status(
        &self,
        id: &str,
        status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        let store = PendingReleaseStore::from_sqlite_services(self);
        PendingReleaseRepository::update_pending_release_status(&store, id, status, grabbed_at)
            .await
    }

    pub async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        let store = PendingReleaseStore::from_sqlite_services(self);
        PendingReleaseRepository::list_standby_pending_releases_for_wanted_item(
            &store,
            wanted_item_id,
        )
        .await
    }

    pub async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        let store = PendingReleaseStore::from_sqlite_services(self);
        PendingReleaseRepository::delete_standby_pending_releases_for_wanted_item(
            &store,
            wanted_item_id,
        )
        .await
    }

    pub async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        let store = PendingReleaseStore::from_sqlite_services(self);
        PendingReleaseRepository::list_all_standby_pending_releases(&store).await
    }

    pub async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: PendingReleaseStatus,
        next_status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        let store = PendingReleaseStore::from_sqlite_services(self);
        PendingReleaseRepository::compare_and_set_pending_release_status(
            &store,
            id,
            current_status,
            next_status,
            grabbed_at,
        )
        .await
    }

    pub async fn supersede_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()> {
        let store = PendingReleaseStore::from_sqlite_services(self);
        PendingReleaseRepository::supersede_pending_releases_for_wanted_item(
            &store,
            wanted_item_id,
            except_id,
        )
        .await
    }

    pub async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        let store = PendingReleaseStore::from_sqlite_services(self);
        PendingReleaseRepository::delete_pending_releases_for_title(&store, title_id).await
    }

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
        let data = match data_json {
            Some(payload) => serde_json::from_str::<
                std::collections::HashMap<String, serde_json::Value>,
            >(&payload)
            .map_err(|error| {
                AppError::Repository(format!("invalid blocklist data_json: {error}"))
            })?,
            None => std::collections::HashMap::new(),
        };
        let entry = NewBlocklistEntry {
            title_id,
            source_title,
            source_hint,
            quality,
            download_id,
            reason,
            data,
        };
        let store = BlocklistStore::from_sqlite_services(self);
        BlocklistRepository::add(&store, &entry).await
    }

    pub async fn list_blocklist_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<BlocklistEntry>> {
        let store = BlocklistStore::from_sqlite_services(self);
        BlocklistRepository::list_for_title(&store, title_id, limit).await
    }
}
