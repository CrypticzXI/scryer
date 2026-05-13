use async_trait::async_trait;
use scryer_application::{
    AppResult, DownloadClientConfigRepository, DownloadClientConfigUpdate, IndexerConfigRepository,
    IndexerConfigUpdate, SubtitleProviderConfigRepository, SubtitleProviderConfigUpdate,
};
use scryer_domain::{DownloadClientConfig, IndexerConfig, SubtitleProviderConfig};
use std::sync::{Arc, RwLock};

use crate::SqliteServices;
use crate::encryption::EncryptionKey;
use crate::queries::{download_client, indexer, subtitle_provider};

#[async_trait]
pub(crate) trait ConfigSql: Clone + Send + Sync + 'static {
    async fn list_indexers(&self, provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>>;
    async fn get_indexer_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>>;
    async fn create_indexer(&self, config: IndexerConfig) -> AppResult<IndexerConfig>;
    async fn touch_indexer_last_error(&self, provider_type: &str) -> AppResult<()>;
    async fn update_indexer(&self, update: IndexerConfigUpdate) -> AppResult<IndexerConfig>;
    async fn delete_indexer(&self, id: &str) -> AppResult<()>;

    async fn list_download_clients(
        &self,
        client_type: Option<String>,
    ) -> AppResult<Vec<DownloadClientConfig>>;
    async fn get_download_client_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>>;
    async fn create_download_client(
        &self,
        config: DownloadClientConfig,
    ) -> AppResult<DownloadClientConfig>;
    async fn update_download_client(
        &self,
        update: DownloadClientConfigUpdate,
    ) -> AppResult<DownloadClientConfig>;
    async fn delete_download_client(&self, id: &str) -> AppResult<()>;
    async fn reorder_download_clients(&self, ordered_ids: Vec<String>) -> AppResult<()>;

    async fn list_subtitle_providers(
        &self,
        provider_type: Option<String>,
    ) -> AppResult<Vec<SubtitleProviderConfig>>;
    async fn get_subtitle_provider_by_id(
        &self,
        id: &str,
    ) -> AppResult<Option<SubtitleProviderConfig>>;
    async fn create_subtitle_provider(
        &self,
        config: SubtitleProviderConfig,
    ) -> AppResult<SubtitleProviderConfig>;
    async fn update_subtitle_provider(
        &self,
        update: SubtitleProviderConfigUpdate,
    ) -> AppResult<SubtitleProviderConfig>;
    async fn delete_subtitle_provider(&self, id: &str) -> AppResult<()>;
}

#[derive(Clone)]
pub struct ConfigStore<S> {
    sql: S,
}

impl<S> ConfigStore<S> {
    pub(crate) fn from_sql(sql: S) -> Self {
        Self { sql }
    }
}

pub type SqliteConfigStore = ConfigStore<SqliteConfigSql>;

#[derive(Clone)]
pub struct SqliteConfigSql {
    db: SqliteServices,
    pool: sqlx::SqlitePool,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl SqliteConfigStore {
    pub fn new(db: &SqliteServices) -> Self {
        Self::from_sql(SqliteConfigSql::new(db))
    }
}

impl SqliteConfigSql {
    fn new(db: &SqliteServices) -> Self {
        Self {
            db: db.clone(),
            pool: db.pool().clone(),
            encryption_key: db.encryption_key_state(),
        }
    }

    fn encryption_key(&self) -> Option<EncryptionKey> {
        self.encryption_key
            .read()
            .ok()
            .and_then(|value| value.clone())
    }
}

#[async_trait]
impl ConfigSql for SqliteConfigSql {
    async fn list_indexers(&self, provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
        let encryption_key = self.encryption_key();
        indexer::list_indexer_configs_query(&self.pool, provider_type, encryption_key.as_ref())
            .await
    }

    async fn get_indexer_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
        let encryption_key = self.encryption_key();
        indexer::get_indexer_config_query(&self.pool, id, encryption_key.as_ref()).await
    }

    async fn create_indexer(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
        self.db.create_indexer_config(config).await
    }

    async fn touch_indexer_last_error(&self, provider_type: &str) -> AppResult<()> {
        self.db.touch_indexer_last_error(provider_type).await
    }

    async fn update_indexer(&self, update: IndexerConfigUpdate) -> AppResult<IndexerConfig> {
        self.db.update_indexer_config(update).await
    }

    async fn delete_indexer(&self, id: &str) -> AppResult<()> {
        self.db.delete_indexer_config(id).await
    }

    async fn list_download_clients(
        &self,
        client_type: Option<String>,
    ) -> AppResult<Vec<DownloadClientConfig>> {
        let encryption_key = self.encryption_key();
        download_client::list_download_client_configs_query(
            &self.pool,
            client_type,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn get_download_client_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
        let encryption_key = self.encryption_key();
        download_client::get_download_client_config_query(&self.pool, id, encryption_key.as_ref())
            .await
    }

    async fn create_download_client(
        &self,
        config: DownloadClientConfig,
    ) -> AppResult<DownloadClientConfig> {
        self.db.create_download_client_config(config).await
    }

    async fn update_download_client(
        &self,
        update: DownloadClientConfigUpdate,
    ) -> AppResult<DownloadClientConfig> {
        self.db.update_download_client_config(update).await
    }

    async fn delete_download_client(&self, id: &str) -> AppResult<()> {
        self.db.delete_download_client_config(id).await
    }

    async fn reorder_download_clients(&self, ordered_ids: Vec<String>) -> AppResult<()> {
        self.db.reorder_download_client_configs(ordered_ids).await
    }

    async fn list_subtitle_providers(
        &self,
        provider_type: Option<String>,
    ) -> AppResult<Vec<SubtitleProviderConfig>> {
        let encryption_key = self.encryption_key();
        subtitle_provider::list_subtitle_provider_configs_query(
            &self.pool,
            provider_type,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn get_subtitle_provider_by_id(
        &self,
        id: &str,
    ) -> AppResult<Option<SubtitleProviderConfig>> {
        let encryption_key = self.encryption_key();
        subtitle_provider::get_subtitle_provider_config_query(
            &self.pool,
            id,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn create_subtitle_provider(
        &self,
        config: SubtitleProviderConfig,
    ) -> AppResult<SubtitleProviderConfig> {
        self.db.create_subtitle_provider_config(config).await
    }

    async fn update_subtitle_provider(
        &self,
        update: SubtitleProviderConfigUpdate,
    ) -> AppResult<SubtitleProviderConfig> {
        self.db.update_subtitle_provider_config(update).await
    }

    async fn delete_subtitle_provider(&self, id: &str) -> AppResult<()> {
        self.db.delete_subtitle_provider_config(id).await
    }
}

#[async_trait]
impl<S: ConfigSql> IndexerConfigRepository for ConfigStore<S> {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
        self.sql.list_indexers(provider_type).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
        self.sql.get_indexer_by_id(id).await
    }

    async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
        self.sql.create_indexer(config).await
    }

    async fn touch_last_error(&self, provider_type: &str) -> AppResult<()> {
        self.sql.touch_indexer_last_error(provider_type).await
    }

    async fn update(&self, update: IndexerConfigUpdate) -> AppResult<IndexerConfig> {
        self.sql.update_indexer(update).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.sql.delete_indexer(id).await
    }
}

#[async_trait]
impl<S: ConfigSql> DownloadClientConfigRepository for ConfigStore<S> {
    async fn list(&self, client_type: Option<String>) -> AppResult<Vec<DownloadClientConfig>> {
        self.sql.list_download_clients(client_type).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
        self.sql.get_download_client_by_id(id).await
    }

    async fn create(&self, config: DownloadClientConfig) -> AppResult<DownloadClientConfig> {
        self.sql.create_download_client(config).await
    }

    async fn update(&self, update: DownloadClientConfigUpdate) -> AppResult<DownloadClientConfig> {
        self.sql.update_download_client(update).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.sql.delete_download_client(id).await
    }

    async fn reorder(&self, ordered_ids: Vec<String>) -> AppResult<()> {
        self.sql.reorder_download_clients(ordered_ids).await
    }
}

#[async_trait]
impl<S: ConfigSql> SubtitleProviderConfigRepository for ConfigStore<S> {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<SubtitleProviderConfig>> {
        self.sql.list_subtitle_providers(provider_type).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<SubtitleProviderConfig>> {
        self.sql.get_subtitle_provider_by_id(id).await
    }

    async fn create(&self, config: SubtitleProviderConfig) -> AppResult<SubtitleProviderConfig> {
        self.sql.create_subtitle_provider(config).await
    }

    async fn update(
        &self,
        update: SubtitleProviderConfigUpdate,
    ) -> AppResult<SubtitleProviderConfig> {
        self.sql.update_subtitle_provider(update).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.sql.delete_subtitle_provider(id).await
    }
}
