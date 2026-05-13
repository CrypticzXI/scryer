use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{AppError, AppResult};
use scryer_domain::{
    Id, NotificationChannelConfig, NotificationEventType, NotificationSubscription,
};
use sqlx::Row;

use crate::encryption::EncryptionKey;
use crate::notification_store::{NotificationSql, NotificationStore};
use crate::postgres::PostgresServices;

pub type PostgresNotificationStore = NotificationStore<PostgresNotificationSql>;

#[derive(Clone)]
pub struct PostgresNotificationSql {
    pool: sqlx::PgPool,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl PostgresNotificationStore {
    pub fn new(db: &PostgresServices) -> Self {
        Self::from_sql(PostgresNotificationSql::new(db))
    }
}

impl PostgresNotificationSql {
    fn new(db: &PostgresServices) -> Self {
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
impl NotificationSql for PostgresNotificationSql {
    async fn list_channels(&self) -> AppResult<Vec<NotificationChannelConfig>> {
        let key = self.encryption_key();
        let rows = sqlx::query(
            "SELECT id, name, channel_type, config_json, is_enabled, created_at, updated_at
             FROM notification_channels
             ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| channel_from_row(row, key.as_ref()))
            .collect()
    }

    async fn get_channel(&self, id: &str) -> AppResult<Option<NotificationChannelConfig>> {
        let key = self.encryption_key();
        let row = sqlx::query(
            "SELECT id, name, channel_type, config_json, is_enabled, created_at, updated_at
             FROM notification_channels
             WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref()
            .map(|row| channel_from_row(row, key.as_ref()))
            .transpose()
    }

    async fn create_channel(
        &self,
        mut config: NotificationChannelConfig,
    ) -> AppResult<NotificationChannelConfig> {
        if config.id.trim().is_empty() {
            config.id = Id::new().0;
        }
        let now = Utc::now();
        config.created_at = now;
        config.updated_at = now;
        upsert_channel(&self.pool, &config, self.encryption_key().as_ref()).await?;
        Ok(config)
    }

    async fn update_channel(
        &self,
        mut config: NotificationChannelConfig,
    ) -> AppResult<NotificationChannelConfig> {
        config.updated_at = Utc::now();
        upsert_channel(&self.pool, &config, self.encryption_key().as_ref()).await?;
        Ok(config)
    }

    async fn delete_channel(&self, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM notification_channels WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    async fn list_subscriptions(&self) -> AppResult<Vec<NotificationSubscription>> {
        let rows = sqlx::query(
            "SELECT id, channel_id, event_type, scope, scope_id, is_enabled, created_at, updated_at
             FROM notification_subscriptions
             ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(subscription_from_row).collect()
    }

    async fn list_subscriptions_for_channel(
        &self,
        channel_id: &str,
    ) -> AppResult<Vec<NotificationSubscription>> {
        let rows = sqlx::query(
            "SELECT id, channel_id, event_type, scope, scope_id, is_enabled, created_at, updated_at
             FROM notification_subscriptions
             WHERE channel_id = $1
             ORDER BY created_at DESC",
        )
        .bind(channel_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(subscription_from_row).collect()
    }

    async fn list_subscriptions_for_event(
        &self,
        event_type: NotificationEventType,
    ) -> AppResult<Vec<NotificationSubscription>> {
        let rows = sqlx::query(
            "SELECT id, channel_id, event_type, scope, scope_id, is_enabled, created_at, updated_at
             FROM notification_subscriptions
             WHERE event_type = $1 AND is_enabled = TRUE
             ORDER BY created_at DESC",
        )
        .bind(event_type.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(subscription_from_row).collect()
    }

    async fn create_subscription(
        &self,
        mut sub: NotificationSubscription,
    ) -> AppResult<NotificationSubscription> {
        if sub.id.trim().is_empty() {
            sub.id = Id::new().0;
        }
        let now = Utc::now();
        sub.created_at = now;
        sub.updated_at = now;
        upsert_subscription(&self.pool, &sub).await?;
        Ok(sub)
    }

    async fn update_subscription(
        &self,
        mut sub: NotificationSubscription,
    ) -> AppResult<NotificationSubscription> {
        sub.updated_at = Utc::now();
        upsert_subscription(&self.pool, &sub).await?;
        Ok(sub)
    }

    async fn delete_subscription(&self, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM notification_subscriptions WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}

async fn upsert_channel(
    pool: &sqlx::PgPool,
    config: &NotificationChannelConfig,
    key: Option<&EncryptionKey>,
) -> AppResult<()> {
    let config_json = maybe_encrypt(key, &config.config_json)?;
    let config_value = config_json_value(key.is_some(), &config_json);
    sqlx::query(
        "INSERT INTO notification_channels
         (id, name, channel_type, config_json, is_enabled, created_at, updated_at)
         VALUES ($1, $2, $3, $4::jsonb, $5, $6, $7)
         ON CONFLICT (id) DO UPDATE SET
            name = EXCLUDED.name,
            channel_type = EXCLUDED.channel_type,
            config_json = EXCLUDED.config_json,
            is_enabled = EXCLUDED.is_enabled,
            updated_at = EXCLUDED.updated_at",
    )
    .bind(&config.id)
    .bind(&config.name)
    .bind(config.channel_type.as_str())
    .bind(config_value)
    .bind(config.is_enabled)
    .bind(config.created_at)
    .bind(config.updated_at)
    .execute(pool)
    .await
    .map_err(repo_err)?;
    Ok(())
}

async fn upsert_subscription(pool: &sqlx::PgPool, sub: &NotificationSubscription) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO notification_subscriptions
         (id, channel_id, event_type, scope, scope_id, is_enabled, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (id) DO UPDATE SET
            channel_id = EXCLUDED.channel_id,
            event_type = EXCLUDED.event_type,
            scope = EXCLUDED.scope,
            scope_id = EXCLUDED.scope_id,
            is_enabled = EXCLUDED.is_enabled,
            updated_at = EXCLUDED.updated_at",
    )
    .bind(&sub.id)
    .bind(&sub.channel_id)
    .bind(sub.event_type.as_str())
    .bind(&sub.scope)
    .bind(&sub.scope_id)
    .bind(sub.is_enabled)
    .bind(sub.created_at)
    .bind(sub.updated_at)
    .execute(pool)
    .await
    .map_err(repo_err)?;
    Ok(())
}

fn channel_from_row(
    row: &sqlx::postgres::PgRow,
    key: Option<&EncryptionKey>,
) -> AppResult<NotificationChannelConfig> {
    let channel_type_raw: String = row.try_get("channel_type").map_err(repo_err)?;
    let channel_type = scryer_domain::ChannelType::parse(&channel_type_raw)
        .ok_or_else(|| AppError::Repository(format!("invalid channel_type: {channel_type_raw}")))?;
    let config_value: serde_json::Value = row.try_get("config_json").map_err(repo_err)?;
    let config_json_raw = match config_value {
        serde_json::Value::String(value) => value,
        value => value.to_string(),
    };
    let config_json = if crate::encryption::is_encrypted(&config_json_raw) {
        if let Some(key) = key {
            crate::encryption::decrypt_value(key, &config_json_raw).map_err(|error| {
                AppError::Repository(format!("failed to decrypt config_json: {error}"))
            })?
        } else {
            config_json_raw
        }
    } else {
        config_json_raw
    };

    Ok(NotificationChannelConfig {
        id: row.try_get("id").map_err(repo_err)?,
        name: row.try_get("name").map_err(repo_err)?,
        channel_type,
        config_json,
        is_enabled: row.try_get("is_enabled").map_err(repo_err)?,
        created_at: row.try_get("created_at").map_err(repo_err)?,
        updated_at: row.try_get("updated_at").map_err(repo_err)?,
    })
}

fn subscription_from_row(row: &sqlx::postgres::PgRow) -> AppResult<NotificationSubscription> {
    let event_type_raw: String = row.try_get("event_type").map_err(repo_err)?;
    let event_type = NotificationEventType::parse(&event_type_raw).ok_or_else(|| {
        AppError::Repository(format!("invalid notification event_type: {event_type_raw}"))
    })?;
    Ok(NotificationSubscription {
        id: row.try_get("id").map_err(repo_err)?,
        channel_id: row.try_get("channel_id").map_err(repo_err)?,
        event_type,
        scope: row.try_get("scope").map_err(repo_err)?,
        scope_id: row.try_get("scope_id").map_err(repo_err)?,
        is_enabled: row.try_get("is_enabled").map_err(repo_err)?,
        created_at: row.try_get("created_at").map_err(repo_err)?,
        updated_at: row.try_get("updated_at").map_err(repo_err)?,
    })
}

fn maybe_encrypt(key: Option<&EncryptionKey>, value: &str) -> AppResult<String> {
    match key {
        Some(key) => crate::encryption::encrypt_value(key, value).map_err(|error| {
            AppError::Repository(format!("failed to encrypt config_json: {error}"))
        }),
        None => Ok(value.to_string()),
    }
}

fn config_json_value(encrypted: bool, value: &str) -> serde_json::Value {
    if encrypted {
        return serde_json::Value::String(value.to_string());
    }

    serde_json::from_str(value).unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}
