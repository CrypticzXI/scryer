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

#[derive(Clone)]
pub struct SqliteConfigStore {
    db: SqliteServices,
    pool: sqlx::SqlitePool,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl SqliteConfigStore {
    pub fn new(db: &SqliteServices) -> Self {
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
        self.db.create_indexer_config(config).await
    }

    async fn touch_last_error(&self, provider_type: &str) -> AppResult<()> {
        self.db.touch_indexer_last_error(provider_type).await
    }

    async fn update(&self, update: IndexerConfigUpdate) -> AppResult<IndexerConfig> {
        self.db.update_indexer_config(update).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.db.delete_indexer_config(id).await
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
        self.db.create_download_client_config(config).await
    }

    async fn update(&self, update: DownloadClientConfigUpdate) -> AppResult<DownloadClientConfig> {
        self.db.update_download_client_config(update).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.db.delete_download_client_config(id).await
    }

    async fn reorder(&self, ordered_ids: Vec<String>) -> AppResult<()> {
        self.db.reorder_download_client_configs(ordered_ids).await
    }
}

#[async_trait]
impl SubtitleProviderConfigRepository for SqliteConfigStore {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<SubtitleProviderConfig>> {
        let encryption_key = self.encryption_key();
        subtitle_provider::list_subtitle_provider_configs_query(
            &self.pool,
            provider_type,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<SubtitleProviderConfig>> {
        let encryption_key = self.encryption_key();
        subtitle_provider::get_subtitle_provider_config_query(
            &self.pool,
            id,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn create(&self, config: SubtitleProviderConfig) -> AppResult<SubtitleProviderConfig> {
        self.db.create_subtitle_provider_config(config).await
    }

    async fn update(
        &self,
        update: SubtitleProviderConfigUpdate,
    ) -> AppResult<SubtitleProviderConfig> {
        self.db.update_subtitle_provider_config(update).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.db.delete_subtitle_provider_config(id).await
    }
}
