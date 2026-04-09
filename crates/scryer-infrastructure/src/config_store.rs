use async_trait::async_trait;
use scryer_application::{
    AppResult, DownloadClientConfigRepository, DownloadClientConfigUpdate, IndexerConfigRepository,
    IndexerConfigUpdate,
};
use scryer_domain::{DownloadClientConfig, IndexerConfig};
use std::sync::{Arc, RwLock};

use crate::SqliteServices;
use crate::encryption::EncryptionKey;
use crate::queries::{download_client, indexer};

#[derive(Clone)]
pub struct SqliteConfigStore {
    pool: sqlx::SqlitePool,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl SqliteConfigStore {
    pub fn new(db: &SqliteServices) -> Self {
        Self {
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
impl IndexerConfigRepository for SqliteConfigStore {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
        let encryption_key = self.encryption_key();
        indexer::list_indexer_configs_query(&self.pool, provider_type, encryption_key.as_ref())
            .await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
        let encryption_key = self.encryption_key();
        indexer::get_indexer_config_query(&self.pool, id, encryption_key.as_ref()).await
    }

    async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
        let encryption_key = self.encryption_key();
        indexer::create_indexer_config_query(&self.pool, &config, encryption_key.as_ref()).await
    }

    async fn touch_last_error(&self, provider_type: &str) -> AppResult<()> {
        indexer::touch_indexer_last_error_query(&self.pool, provider_type).await
    }

    async fn update(&self, update: IndexerConfigUpdate) -> AppResult<IndexerConfig> {
        let encryption_key = self.encryption_key();
        indexer::update_indexer_config_query(&self.pool, &update, encryption_key.as_ref()).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        indexer::delete_indexer_config_query(&self.pool, id).await
    }
}

#[async_trait]
impl DownloadClientConfigRepository for SqliteConfigStore {
    async fn list(&self, client_type: Option<String>) -> AppResult<Vec<DownloadClientConfig>> {
        let encryption_key = self.encryption_key();
        download_client::list_download_client_configs_query(
            &self.pool,
            client_type,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
        let encryption_key = self.encryption_key();
        download_client::get_download_client_config_query(&self.pool, id, encryption_key.as_ref())
            .await
    }

    async fn create(&self, config: DownloadClientConfig) -> AppResult<DownloadClientConfig> {
        let encryption_key = self.encryption_key();
        download_client::create_download_client_config_query(
            &self.pool,
            &config,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn update(&self, update: DownloadClientConfigUpdate) -> AppResult<DownloadClientConfig> {
        let encryption_key = self.encryption_key();
        download_client::update_download_client_config_query(
            &self.pool,
            &update,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        download_client::delete_download_client_config_query(&self.pool, id).await
    }

    async fn reorder(&self, ordered_ids: Vec<String>) -> AppResult<()> {
        download_client::reorder_download_client_configs_query(&self.pool, &ordered_ids).await
    }
}
