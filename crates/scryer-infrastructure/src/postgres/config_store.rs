use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, DownloadClientConfigUpdate, IndexerConfigUpdate,
    SubtitleProviderConfigUpdate,
};
use scryer_domain::{
    DownloadClientConfig, DownloadClientStatus, IndexerConfig, SubtitleProviderConfig,
};
use serde_json::Value;
use sqlx::{Executor, Postgres, Row};

use crate::config_store::{ConfigSql, ConfigStore};
use crate::encryption::EncryptionKey;

pub type PostgresConfigStore = ConfigStore<PostgresConfigSql>;

#[derive(Clone)]
pub struct PostgresConfigSql {
    pool: sqlx::PgPool,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl PostgresConfigStore {
    pub fn new(db: &super::PostgresServices) -> Self {
        Self::from_sql(PostgresConfigSql::new(
            db.pool().clone(),
            db.encryption_key_state(),
        ))
    }
}

impl PostgresConfigSql {
    fn new(pool: sqlx::PgPool, encryption_key: Arc<RwLock<Option<EncryptionKey>>>) -> Self {
        Self {
            pool,
            encryption_key,
        }
    }

    fn encryption_key(&self) -> Option<EncryptionKey> {
        self.encryption_key
            .read()
            .ok()
            .and_then(|value| value.clone())
    }

    async fn upsert_indexer_config(&self, config: &IndexerConfig) -> AppResult<()> {
        let encryption_key = self.encryption_key();
        upsert_indexer_config_with(&self.pool, config, encryption_key.as_ref()).await
    }

    async fn upsert_download_client_config(&self, config: &DownloadClientConfig) -> AppResult<()> {
        let encryption_key = self.encryption_key();
        upsert_download_client_config_with(&self.pool, config, encryption_key.as_ref()).await
    }

    async fn upsert_subtitle_provider_config(
        &self,
        config: &SubtitleProviderConfig,
    ) -> AppResult<()> {
        let encryption_key = self.encryption_key();
        upsert_subtitle_provider_config_with(&self.pool, config, encryption_key.as_ref()).await
    }
}

#[async_trait]
impl ConfigSql for PostgresConfigSql {
    async fn list_indexers(&self, provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
        let rows = if let Some(provider_type) = provider_type {
            sqlx::query(&format!(
                "SELECT {INDEXER_COLUMNS} FROM indexers WHERE provider_type = $1 ORDER BY created_at DESC"
            ))
            .bind(provider_type)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(&format!(
                "SELECT {INDEXER_COLUMNS} FROM indexers ORDER BY created_at DESC"
            ))
            .fetch_all(&self.pool)
            .await
        }
        .map_err(repo_err)?;

        let encryption_key = self.encryption_key();
        rows.iter()
            .map(|row| row_to_indexer_config(row, encryption_key.as_ref()))
            .collect()
    }

    async fn get_indexer_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
        let row = sqlx::query(&format!(
            "SELECT {INDEXER_COLUMNS} FROM indexers WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        let encryption_key = self.encryption_key();
        row.as_ref()
            .map(|row| row_to_indexer_config(row, encryption_key.as_ref()))
            .transpose()
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
        sqlx::query(
            "UPDATE indexers
                SET last_error_at = $2,
                    last_health_status = 'error',
                    updated_at = $2
              WHERE provider_type = $1",
        )
        .bind(provider_type)
        .bind(Utc::now())
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn update_indexer(&self, update: IndexerConfigUpdate) -> AppResult<IndexerConfig> {
        let mut config = self
            .get_indexer_by_id(&update.id)
            .await?
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
        self.upsert_indexer_config(&config).await?;
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
            sqlx::query(&format!(
                "SELECT {DOWNLOAD_CLIENT_COLUMNS} FROM download_clients WHERE client_type = $1 ORDER BY client_priority ASC"
            ))
            .bind(client_type)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(&format!(
                "SELECT {DOWNLOAD_CLIENT_COLUMNS} FROM download_clients ORDER BY client_priority ASC"
            ))
            .fetch_all(&self.pool)
            .await
        }
        .map_err(repo_err)?;

        let encryption_key = self.encryption_key();
        rows.iter()
            .map(|row| row_to_download_client_config(row, encryption_key.as_ref()))
            .collect()
    }

    async fn get_download_client_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
        let row = sqlx::query(&format!(
            "SELECT {DOWNLOAD_CLIENT_COLUMNS} FROM download_clients WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        let encryption_key = self.encryption_key();
        row.as_ref()
            .map(|row| row_to_download_client_config(row, encryption_key.as_ref()))
            .transpose()
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
        let mut config = self
            .get_download_client_by_id(&update.id)
            .await?
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
        self.upsert_download_client_config(&config).await?;
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
            sqlx::query(
                "UPDATE download_clients
                    SET client_priority = $2,
                        updated_at = $3
                  WHERE id = $1",
            )
            .bind(id)
            .bind(index as i64)
            .bind(Utc::now())
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
            sqlx::query(&format!(
                "SELECT {SUBTITLE_PROVIDER_COLUMNS} FROM subtitle_provider_configs WHERE provider_type = $1 ORDER BY created_at DESC"
            ))
            .bind(provider_type)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(&format!(
                "SELECT {SUBTITLE_PROVIDER_COLUMNS} FROM subtitle_provider_configs ORDER BY created_at DESC"
            ))
            .fetch_all(&self.pool)
            .await
        }
        .map_err(repo_err)?;

        let encryption_key = self.encryption_key();
        rows.iter()
            .map(|row| row_to_subtitle_provider_config(row, encryption_key.as_ref()))
            .collect()
    }

    async fn get_subtitle_provider_by_id(
        &self,
        id: &str,
    ) -> AppResult<Option<SubtitleProviderConfig>> {
        let row = sqlx::query(&format!(
            "SELECT {SUBTITLE_PROVIDER_COLUMNS} FROM subtitle_provider_configs WHERE id = $1"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        let encryption_key = self.encryption_key();
        row.as_ref()
            .map(|row| row_to_subtitle_provider_config(row, encryption_key.as_ref()))
            .transpose()
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
        let mut config = self
            .get_subtitle_provider_by_id(&update.id)
            .await?
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
        self.upsert_subtitle_provider_config(&config).await?;
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

const INDEXER_COLUMNS: &str =
    "id, name, provider_type, base_url, api_key_encrypted, rate_limit_seconds,
    rate_limit_burst, disabled_until, is_enabled, enable_interactive_search, enable_auto_search,
    managed_parent_config_id, managed_child_key, managed_metadata_json, last_health_status,
    last_error_at, config_json, created_at, updated_at";

const DOWNLOAD_CLIENT_COLUMNS: &str = "id, name, client_type, config_json, is_enabled, status,
    client_priority, last_error, last_seen_at, created_at, updated_at";

const SUBTITLE_PROVIDER_COLUMNS: &str = "id, name, provider_type, config_json, is_enabled,
    enabled_facets, last_health_status, last_error, last_error_at, disabled_until,
    created_at, updated_at";

async fn upsert_indexer_config_with<'e, E>(
    executor: E,
    config: &IndexerConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<()>
where
    E: Executor<'e, Database = Postgres>,
{
    let stored_api_key = maybe_encrypt_optional(encryption_key, config.api_key_encrypted.as_ref())?;
    let stored_config_json = maybe_encrypt_optional(encryption_key, config.config_json.as_ref())?;
    sqlx::query(
        "INSERT INTO indexers (
            id, name, provider_type, base_url, api_key_encrypted, rate_limit_seconds,
            rate_limit_burst, disabled_until, is_enabled, enable_interactive_search,
            enable_auto_search, managed_parent_config_id, managed_child_key,
            managed_metadata_json, last_health_status, last_error_at, config_json,
            created_at, updated_at
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                 $11, $12, $13, $14, $15, $16, $17, $18, $19)
         ON CONFLICT (id) DO UPDATE SET
            name = EXCLUDED.name,
            provider_type = EXCLUDED.provider_type,
            base_url = EXCLUDED.base_url,
            api_key_encrypted = EXCLUDED.api_key_encrypted,
            rate_limit_seconds = EXCLUDED.rate_limit_seconds,
            rate_limit_burst = EXCLUDED.rate_limit_burst,
            disabled_until = EXCLUDED.disabled_until,
            is_enabled = EXCLUDED.is_enabled,
            enable_interactive_search = EXCLUDED.enable_interactive_search,
            enable_auto_search = EXCLUDED.enable_auto_search,
            managed_parent_config_id = EXCLUDED.managed_parent_config_id,
            managed_child_key = EXCLUDED.managed_child_key,
            managed_metadata_json = EXCLUDED.managed_metadata_json,
            last_health_status = EXCLUDED.last_health_status,
            last_error_at = EXCLUDED.last_error_at,
            config_json = EXCLUDED.config_json,
            updated_at = EXCLUDED.updated_at",
    )
    .bind(&config.id)
    .bind(&config.name)
    .bind(&config.provider_type)
    .bind(&config.base_url)
    .bind(&stored_api_key)
    .bind(config.rate_limit_seconds)
    .bind(config.rate_limit_burst)
    .bind(config.disabled_until)
    .bind(config.is_enabled)
    .bind(config.enable_interactive_search)
    .bind(config.enable_auto_search)
    .bind(&config.managed_parent_config_id)
    .bind(&config.managed_child_key)
    .bind(&config.managed_metadata_json)
    .bind(&config.last_health_status)
    .bind(config.last_error_at)
    .bind(&stored_config_json)
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
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<()>
where
    E: Executor<'e, Database = Postgres>,
{
    let stored_config_json = maybe_encrypt_value(encryption_key, &config.config_json)?;
    sqlx::query(
        "INSERT INTO download_clients (
            id, name, client_type, config_json, is_enabled, status,
            client_priority, last_error, last_seen_at, created_at, updated_at
         )
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (id) DO UPDATE SET
            name = EXCLUDED.name,
            client_type = EXCLUDED.client_type,
            config_json = EXCLUDED.config_json,
            is_enabled = EXCLUDED.is_enabled,
            status = EXCLUDED.status,
            client_priority = EXCLUDED.client_priority,
            last_error = EXCLUDED.last_error,
            last_seen_at = EXCLUDED.last_seen_at,
            updated_at = EXCLUDED.updated_at",
    )
    .bind(&config.id)
    .bind(&config.name)
    .bind(&config.client_type)
    .bind(&stored_config_json)
    .bind(config.is_enabled)
    .bind(config.status.as_str())
    .bind(config.client_priority)
    .bind(&config.last_error)
    .bind(config.last_seen_at)
    .bind(config.created_at)
    .bind(config.updated_at)
    .execute(executor)
    .await
    .map_err(repo_err)?;
    Ok(())
}

async fn upsert_subtitle_provider_config_with<'e, E>(
    executor: E,
    config: &SubtitleProviderConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<()>
where
    E: Executor<'e, Database = Postgres>,
{
    let stored_config_json = maybe_encrypt_value(encryption_key, &config.config_json)?;
    let enabled_facets = serde_json::to_value(&config.enabled_facets).map_err(repo_err)?;
    sqlx::query(
        "INSERT INTO subtitle_provider_configs (
            id, name, provider_type, config_json, is_enabled, enabled_facets,
            last_health_status, last_error, last_error_at, disabled_until, created_at, updated_at
         )
         VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (id) DO UPDATE SET
            name = EXCLUDED.name,
            provider_type = EXCLUDED.provider_type,
            config_json = EXCLUDED.config_json,
            is_enabled = EXCLUDED.is_enabled,
            enabled_facets = EXCLUDED.enabled_facets,
            last_health_status = EXCLUDED.last_health_status,
            last_error = EXCLUDED.last_error,
            last_error_at = EXCLUDED.last_error_at,
            disabled_until = EXCLUDED.disabled_until,
            updated_at = EXCLUDED.updated_at",
    )
    .bind(&config.id)
    .bind(&config.name)
    .bind(&config.provider_type)
    .bind(&stored_config_json)
    .bind(config.is_enabled)
    .bind(enabled_facets)
    .bind(&config.last_health_status)
    .bind(&config.last_error)
    .bind(config.last_error_at)
    .bind(config.disabled_until)
    .bind(config.created_at)
    .bind(config.updated_at)
    .execute(executor)
    .await
    .map_err(repo_err)?;
    Ok(())
}

fn row_to_indexer_config(
    row: &sqlx::postgres::PgRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<IndexerConfig> {
    let api_key_encrypted = decrypt_optional_value(
        encryption_key,
        row.try_get("api_key_encrypted").map_err(repo_err)?,
        "API key",
    )?;
    let config_json = decrypt_optional_value(
        encryption_key,
        row.try_get("config_json").map_err(repo_err)?,
        "config_json",
    )?;
    Ok(IndexerConfig {
        id: row.try_get("id").map_err(repo_err)?,
        name: row.try_get("name").map_err(repo_err)?,
        provider_type: row.try_get("provider_type").map_err(repo_err)?,
        base_url: row.try_get("base_url").map_err(repo_err)?,
        api_key_encrypted,
        rate_limit_seconds: row.try_get("rate_limit_seconds").map_err(repo_err)?,
        rate_limit_burst: row.try_get("rate_limit_burst").map_err(repo_err)?,
        disabled_until: row.try_get("disabled_until").map_err(repo_err)?,
        is_enabled: row.try_get("is_enabled").map_err(repo_err)?,
        enable_interactive_search: row.try_get("enable_interactive_search").map_err(repo_err)?,
        enable_auto_search: row.try_get("enable_auto_search").map_err(repo_err)?,
        managed_parent_config_id: row.try_get("managed_parent_config_id").map_err(repo_err)?,
        managed_child_key: row.try_get("managed_child_key").map_err(repo_err)?,
        managed_metadata_json: row.try_get("managed_metadata_json").map_err(repo_err)?,
        last_health_status: row.try_get("last_health_status").map_err(repo_err)?,
        last_error_at: row.try_get("last_error_at").map_err(repo_err)?,
        config_json,
        created_at: row.try_get("created_at").map_err(repo_err)?,
        updated_at: row.try_get("updated_at").map_err(repo_err)?,
    })
}

fn row_to_download_client_config(
    row: &sqlx::postgres::PgRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<DownloadClientConfig> {
    let config_json = decrypt_value(
        encryption_key,
        row.try_get("config_json").map_err(repo_err)?,
        "config_json",
    )?;
    let status_raw: String = row.try_get("status").map_err(repo_err)?;
    Ok(DownloadClientConfig {
        id: row.try_get("id").map_err(repo_err)?,
        name: row.try_get("name").map_err(repo_err)?,
        client_type: row.try_get("client_type").map_err(repo_err)?,
        config_json,
        client_priority: row.try_get("client_priority").map_err(repo_err)?,
        is_enabled: row.try_get("is_enabled").map_err(repo_err)?,
        status: DownloadClientStatus::parse(&status_raw).unwrap_or_default(),
        last_error: row.try_get("last_error").map_err(repo_err)?,
        last_seen_at: row.try_get("last_seen_at").map_err(repo_err)?,
        created_at: row.try_get("created_at").map_err(repo_err)?,
        updated_at: row.try_get("updated_at").map_err(repo_err)?,
    })
}

fn row_to_subtitle_provider_config(
    row: &sqlx::postgres::PgRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<SubtitleProviderConfig> {
    let config_json = decrypt_value(
        encryption_key,
        row.try_get("config_json").map_err(repo_err)?,
        "config_json",
    )?;
    let enabled_facets_value: Value = row.try_get("enabled_facets").map_err(repo_err)?;
    let enabled_facets = serde_json::from_value(enabled_facets_value).unwrap_or_default();
    Ok(SubtitleProviderConfig {
        id: row.try_get("id").map_err(repo_err)?,
        name: row.try_get("name").map_err(repo_err)?,
        provider_type: row.try_get("provider_type").map_err(repo_err)?,
        config_json,
        enabled_facets,
        is_enabled: row.try_get("is_enabled").map_err(repo_err)?,
        last_health_status: row.try_get("last_health_status").map_err(repo_err)?,
        last_error: row.try_get("last_error").map_err(repo_err)?,
        last_error_at: row.try_get("last_error_at").map_err(repo_err)?,
        disabled_until: row.try_get("disabled_until").map_err(repo_err)?,
        created_at: row.try_get("created_at").map_err(repo_err)?,
        updated_at: row.try_get("updated_at").map_err(repo_err)?,
    })
}

fn maybe_encrypt_optional(
    key: Option<&EncryptionKey>,
    value: Option<&String>,
) -> AppResult<Option<String>> {
    value
        .map(|value| maybe_encrypt_value(key, value))
        .transpose()
}

fn maybe_encrypt_value(key: Option<&EncryptionKey>, value: &str) -> AppResult<String> {
    let Some(key) = key else {
        return Ok(value.to_string());
    };
    crate::encryption::encrypt_value(key, value)
        .map_err(|error| AppError::Repository(format!("failed to encrypt config_json: {error}")))
}

fn decrypt_optional_value(
    key: Option<&EncryptionKey>,
    value: Option<String>,
    label: &str,
) -> AppResult<Option<String>> {
    value
        .map(|value| decrypt_value(key, value, label))
        .transpose()
}

fn decrypt_value(key: Option<&EncryptionKey>, value: String, label: &str) -> AppResult<String> {
    if !crate::encryption::is_encrypted(&value) {
        return Ok(value);
    }
    let Some(key) = key else {
        return Ok(value);
    };
    crate::encryption::decrypt_value(key, &value)
        .map_err(|error| AppError::Repository(format!("failed to decrypt {label}: {error}")))
}

fn repo_err(error: impl ToString) -> AppError {
    AppError::Repository(error.to_string())
}
