use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, DownloadClientConfigUpdate, IndexerConfigUpdate,
    SubtitleProviderConfigUpdate,
};
use scryer_domain::{DownloadClientConfig, IndexerConfig, SubtitleProviderConfig};
use serde::de::DeserializeOwned;
use serde_json::Value;
use sqlx::{Executor, Postgres, Row};

use crate::config_store::{ConfigSql, ConfigStore};

pub type PostgresConfigStore = ConfigStore<PostgresConfigSql>;

#[derive(Clone)]
pub struct PostgresConfigSql {
    pool: sqlx::PgPool,
}

impl PostgresConfigStore {
    pub fn new(db: &super::PostgresServices) -> Self {
        Self::from_sql(PostgresConfigSql::new(db.pool().clone()))
    }
}

impl PostgresConfigSql {
    fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    async fn upsert_indexer_config(&self, config: &IndexerConfig) -> AppResult<()> {
        upsert_indexer_config_with(&self.pool, config).await
    }

    async fn upsert_download_client_config(&self, config: &DownloadClientConfig) -> AppResult<()> {
        upsert_download_client_config_with(&self.pool, config).await
    }

    async fn upsert_subtitle_provider_config(
        &self,
        config: &SubtitleProviderConfig,
    ) -> AppResult<()> {
        upsert_subtitle_provider_config_with(&self.pool, config).await
    }
}

#[async_trait]
impl ConfigSql for PostgresConfigSql {
    async fn list_indexers(&self, provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
        let rows = if let Some(provider_type) = provider_type {
            sqlx::query(
                "SELECT record_json FROM indexers WHERE provider_type = $1 ORDER BY name, id",
            )
            .bind(provider_type)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query("SELECT record_json FROM indexers ORDER BY name, id")
                .fetch_all(&self.pool)
                .await
        }
        .map_err(repo_err)?;

        rows.iter().map(record_from_row).collect()
    }

    async fn get_indexer_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
        let row = sqlx::query("SELECT record_json FROM indexers WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(record_from_row).transpose()
    }

    async fn create_indexer(&self, mut config: IndexerConfig) -> AppResult<IndexerConfig> {
        let now = Utc::now();
        config.created_at = if config.created_at.timestamp() == 0 {
            now
        } else {
            config.created_at
        };
        config.updated_at = now;
        self.upsert_indexer_config(&config).await?;
        Ok(config)
    }

    async fn touch_indexer_last_error(&self, provider_type: &str) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let rows = sqlx::query(
            "SELECT record_json FROM indexers WHERE provider_type = $1 ORDER BY name, id",
        )
        .bind(provider_type)
        .fetch_all(&mut *tx)
        .await
        .map_err(repo_err)?;

        for row in rows {
            let mut config: IndexerConfig = record_from_row(&row)?;
            config.last_error_at = Some(Utc::now());
            config.last_health_status = Some("error".to_string());
            config.updated_at = Utc::now();
            upsert_indexer_config_with(&mut *tx, &config).await?;
        }
        tx.commit().await.map_err(repo_err)?;
        Ok(())
    }

    async fn update_indexer(&self, update: IndexerConfigUpdate) -> AppResult<IndexerConfig> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let row = sqlx::query("SELECT record_json FROM indexers WHERE id = $1")
            .bind(&update.id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(repo_err)?;
        let mut config: IndexerConfig = row
            .as_ref()
            .map(record_from_row)
            .transpose()?
            .ok_or_else(|| AppError::Repository("indexer config was not found".to_string()))?;

        if let Some(name) = update.name {
            config.name = name;
        }
        if let Some(provider_type) = update.provider_type {
            config.provider_type = provider_type;
        }
        if let Some(base_url) = update.derived_base_url {
            config.base_url = base_url;
        }
        if let Some(rate_limit_seconds) = update.rate_limit_seconds {
            config.rate_limit_seconds = Some(rate_limit_seconds);
        }
        if let Some(rate_limit_burst) = update.rate_limit_burst {
            config.rate_limit_burst = Some(rate_limit_burst);
        }
        if let Some(is_enabled) = update.is_enabled {
            config.is_enabled = is_enabled;
        }
        if let Some(enable_interactive_search) = update.enable_interactive_search {
            config.enable_interactive_search = enable_interactive_search;
        }
        if let Some(enable_auto_search) = update.enable_auto_search {
            config.enable_auto_search = enable_auto_search;
        }
        if let Some(managed_parent_config_id) = update.managed_parent_config_id {
            config.managed_parent_config_id = managed_parent_config_id;
        }
        if let Some(managed_child_key) = update.managed_child_key {
            config.managed_child_key = managed_child_key;
        }
        if let Some(managed_metadata_json) = update.managed_metadata_json {
            config.managed_metadata_json = managed_metadata_json;
        }
        if let Some(config_json) = update.config_json {
            config.config_json = Some(config_json);
        }
        config.updated_at = Utc::now();
        upsert_indexer_config_with(&mut *tx, &config).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(config)
    }

    async fn delete_indexer(&self, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM indexers WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    async fn list_download_clients(
        &self,
        client_type: Option<String>,
    ) -> AppResult<Vec<DownloadClientConfig>> {
        let rows = if let Some(client_type) = client_type {
            sqlx::query(
                "SELECT record_json FROM download_clients WHERE client_type = $1 ORDER BY client_priority, name, id",
            )
            .bind(client_type)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(
                "SELECT record_json FROM download_clients ORDER BY client_priority, name, id",
            )
            .fetch_all(&self.pool)
            .await
        }
        .map_err(repo_err)?;

        rows.iter().map(record_from_row).collect()
    }

    async fn get_download_client_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
        let row = sqlx::query("SELECT record_json FROM download_clients WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(record_from_row).transpose()
    }

    async fn create_download_client(
        &self,
        mut config: DownloadClientConfig,
    ) -> AppResult<DownloadClientConfig> {
        let now = Utc::now();
        config.created_at = if config.created_at.timestamp() == 0 {
            now
        } else {
            config.created_at
        };
        config.updated_at = now;
        self.upsert_download_client_config(&config).await?;
        Ok(config)
    }

    async fn update_download_client(
        &self,
        update: DownloadClientConfigUpdate,
    ) -> AppResult<DownloadClientConfig> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let row = sqlx::query("SELECT record_json FROM download_clients WHERE id = $1")
            .bind(&update.id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(repo_err)?;
        let mut config: DownloadClientConfig = row
            .as_ref()
            .map(record_from_row)
            .transpose()?
            .ok_or_else(|| {
                AppError::Repository("download client config was not found".to_string())
            })?;
        if let Some(name) = update.name {
            config.name = name;
        }
        if let Some(client_type) = update.client_type {
            config.client_type = client_type;
        }
        if let Some(config_json) = update.config_json {
            config.config_json = config_json;
        }
        if let Some(is_enabled) = update.is_enabled {
            config.is_enabled = is_enabled;
        }
        config.updated_at = Utc::now();
        upsert_download_client_config_with(&mut *tx, &config).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(config)
    }

    async fn delete_download_client(&self, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM download_clients WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    async fn reorder_download_clients(&self, ordered_ids: Vec<String>) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        for (index, id) in ordered_ids.iter().enumerate() {
            let row = sqlx::query("SELECT record_json FROM download_clients WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(repo_err)?;
            let config: Option<DownloadClientConfig> =
                row.as_ref().map(record_from_row).transpose()?;
            let Some(mut config) = config else {
                continue;
            };
            config.client_priority = index as i64;
            config.updated_at = Utc::now();
            let config_json = json_value(&config.config_json)?;
            let record_json = serde_json::to_value(&config).map_err(repo_err)?;
            sqlx::query(
                "UPDATE download_clients
                    SET client_priority = $2, config_json = $3::jsonb, record_json = $4::jsonb, updated_at = $5
                  WHERE id = $1",
            )
            .bind(id)
            .bind(config.client_priority)
            .bind(config_json)
            .bind(record_json)
            .bind(config.updated_at)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        }
        tx.commit().await.map_err(repo_err)?;
        Ok(())
    }

    async fn list_subtitle_providers(
        &self,
        provider_type: Option<String>,
    ) -> AppResult<Vec<SubtitleProviderConfig>> {
        let rows = if let Some(provider_type) = provider_type {
            sqlx::query(
                "SELECT record_json FROM subtitle_provider_configs WHERE provider_type = $1 ORDER BY name, id",
            )
            .bind(provider_type)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query("SELECT record_json FROM subtitle_provider_configs ORDER BY name, id")
                .fetch_all(&self.pool)
                .await
        }
        .map_err(repo_err)?;

        rows.iter().map(record_from_row).collect()
    }

    async fn get_subtitle_provider_by_id(
        &self,
        id: &str,
    ) -> AppResult<Option<SubtitleProviderConfig>> {
        let row = sqlx::query("SELECT record_json FROM subtitle_provider_configs WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(record_from_row).transpose()
    }

    async fn create_subtitle_provider(
        &self,
        mut config: SubtitleProviderConfig,
    ) -> AppResult<SubtitleProviderConfig> {
        let now = Utc::now();
        config.created_at = if config.created_at.timestamp() == 0 {
            now
        } else {
            config.created_at
        };
        config.updated_at = now;
        self.upsert_subtitle_provider_config(&config).await?;
        Ok(config)
    }

    async fn update_subtitle_provider(
        &self,
        update: SubtitleProviderConfigUpdate,
    ) -> AppResult<SubtitleProviderConfig> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let row = sqlx::query("SELECT record_json FROM subtitle_provider_configs WHERE id = $1")
            .bind(&update.id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(repo_err)?;
        let mut config: SubtitleProviderConfig = row
            .as_ref()
            .map(record_from_row)
            .transpose()?
            .ok_or_else(|| {
                AppError::Repository("subtitle provider config was not found".to_string())
            })?;
        if let Some(name) = update.name {
            config.name = name;
        }
        if let Some(provider_type) = update.provider_type {
            config.provider_type = provider_type;
        }
        if let Some(config_json) = update.config_json {
            config.config_json = config_json;
        }
        if let Some(enabled_facets) = update.enabled_facets {
            config.enabled_facets = enabled_facets;
        }
        if let Some(is_enabled) = update.is_enabled {
            config.is_enabled = is_enabled;
        }
        if let Some(last_health_status) = update.last_health_status {
            config.last_health_status = Some(last_health_status);
        }
        if let Some(last_error) = update.last_error {
            config.last_error = last_error;
        }
        if let Some(last_error_at) = update.last_error_at {
            config.last_error_at = last_error_at;
        }
        if let Some(disabled_until) = update.disabled_until {
            config.disabled_until = disabled_until;
        }
        config.updated_at = Utc::now();
        upsert_subtitle_provider_config_with(&mut *tx, &config).await?;
        tx.commit().await.map_err(repo_err)?;
        Ok(config)
    }

    async fn delete_subtitle_provider(&self, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM subtitle_provider_configs WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}

async fn upsert_indexer_config_with<'e, E>(executor: E, config: &IndexerConfig) -> AppResult<()>
where
    E: Executor<'e, Database = Postgres>,
{
    let config_json = optional_json_value(config.config_json.as_deref())?;
    let record_json = serde_json::to_value(config).map_err(repo_err)?;
    sqlx::query(
        "INSERT INTO indexers (
            id, name, provider_type, base_url, api_key, config_json, record_json,
            is_enabled, status, last_error, last_seen_at, created_at, updated_at
         )
         VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7::jsonb, $8, $9, $10, $11, $12, $13)
         ON CONFLICT (id) DO UPDATE SET
            name = EXCLUDED.name,
            provider_type = EXCLUDED.provider_type,
            base_url = EXCLUDED.base_url,
            api_key = EXCLUDED.api_key,
            config_json = EXCLUDED.config_json,
            record_json = EXCLUDED.record_json,
            is_enabled = EXCLUDED.is_enabled,
            status = EXCLUDED.status,
            last_error = EXCLUDED.last_error,
            last_seen_at = EXCLUDED.last_seen_at,
            updated_at = EXCLUDED.updated_at",
    )
    .bind(&config.id)
    .bind(&config.name)
    .bind(&config.provider_type)
    .bind(&config.base_url)
    .bind(&config.api_key_encrypted)
    .bind(config_json)
    .bind(record_json)
    .bind(config.is_enabled)
    .bind(config.last_health_status.as_deref().unwrap_or("unknown"))
    .bind(config.last_health_status.clone())
    .bind(config.last_error_at)
    .bind(config.created_at)
    .bind(config.updated_at)
    .execute(executor)
    .await
    .map_err(repo_err)?;
    Ok(())
}

async fn upsert_download_client_config_with<'e, E>(
    executor: E,
    config: &DownloadClientConfig,
) -> AppResult<()>
where
    E: Executor<'e, Database = Postgres>,
{
    let config_json = json_value(&config.config_json)?;
    let record_json = serde_json::to_value(config).map_err(repo_err)?;
    sqlx::query(
        "INSERT INTO download_clients (
            id, name, client_type, config_json, record_json, is_enabled, status,
            last_error, last_seen_at, created_at, updated_at, client_priority
         )
         VALUES ($1, $2, $3, $4::jsonb, $5::jsonb, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (id) DO UPDATE SET
            name = EXCLUDED.name,
            client_type = EXCLUDED.client_type,
            config_json = EXCLUDED.config_json,
            record_json = EXCLUDED.record_json,
            is_enabled = EXCLUDED.is_enabled,
            status = EXCLUDED.status,
            last_error = EXCLUDED.last_error,
            last_seen_at = EXCLUDED.last_seen_at,
            updated_at = EXCLUDED.updated_at,
            client_priority = EXCLUDED.client_priority",
    )
    .bind(&config.id)
    .bind(&config.name)
    .bind(&config.client_type)
    .bind(config_json)
    .bind(record_json)
    .bind(config.is_enabled)
    .bind(config.status.as_str())
    .bind(&config.last_error)
    .bind(config.last_seen_at)
    .bind(config.created_at)
    .bind(config.updated_at)
    .bind(config.client_priority)
    .execute(executor)
    .await
    .map_err(repo_err)?;
    Ok(())
}

async fn upsert_subtitle_provider_config_with<'e, E>(
    executor: E,
    config: &SubtitleProviderConfig,
) -> AppResult<()>
where
    E: Executor<'e, Database = Postgres>,
{
    let config_json = json_value(&config.config_json)?;
    let record_json = serde_json::to_value(config).map_err(repo_err)?;
    sqlx::query(
        "INSERT INTO subtitle_provider_configs (
            id, name, provider_type, config_json, record_json, is_enabled, status,
            last_error, last_seen_at, created_at, updated_at
         )
         VALUES ($1, $2, $3, $4::jsonb, $5::jsonb, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (id) DO UPDATE SET
            name = EXCLUDED.name,
            provider_type = EXCLUDED.provider_type,
            config_json = EXCLUDED.config_json,
            record_json = EXCLUDED.record_json,
            is_enabled = EXCLUDED.is_enabled,
            status = EXCLUDED.status,
            last_error = EXCLUDED.last_error,
            last_seen_at = EXCLUDED.last_seen_at,
            updated_at = EXCLUDED.updated_at",
    )
    .bind(&config.id)
    .bind(&config.name)
    .bind(&config.provider_type)
    .bind(config_json)
    .bind(record_json)
    .bind(config.is_enabled)
    .bind(config.last_health_status.as_deref().unwrap_or("unknown"))
    .bind(&config.last_error)
    .bind(config.last_error_at)
    .bind(config.created_at)
    .bind(config.updated_at)
    .execute(executor)
    .await
    .map_err(repo_err)?;
    Ok(())
}

fn record_from_row<T: DeserializeOwned>(row: &sqlx::postgres::PgRow) -> AppResult<T> {
    let value: Value = row.try_get("record_json").map_err(repo_err)?;
    serde_json::from_value(value).map_err(repo_err)
}

fn optional_json_value(raw: Option<&str>) -> AppResult<Option<Value>> {
    raw.map(json_value).transpose()
}

fn json_value(raw: &str) -> AppResult<Value> {
    serde_json::from_str(raw)
        .map_err(|error| AppError::Validation(format!("invalid logical JSON value: {error}")))
}

fn repo_err(error: impl ToString) -> AppError {
    AppError::Repository(error.to_string())
}
