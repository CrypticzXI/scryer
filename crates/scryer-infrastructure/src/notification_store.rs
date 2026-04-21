use async_trait::async_trait;
use scryer_application::{
    AppResult, NotificationChannelRepository, NotificationSubscriptionRepository,
};
use scryer_domain::{NotificationChannelConfig, NotificationEventType, NotificationSubscription};
use std::sync::{Arc, RwLock};

use crate::SqliteServices;
use crate::encryption::EncryptionKey;
use crate::queries::{notification_channel, notification_subscription};

#[derive(Clone)]
pub struct SqliteNotificationStore {
    db: SqliteServices,
    pool: sqlx::SqlitePool,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl SqliteNotificationStore {
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
impl NotificationChannelRepository for SqliteNotificationStore {
    async fn list_channels(&self) -> AppResult<Vec<NotificationChannelConfig>> {
        let encryption_key = self.encryption_key();
        notification_channel::list_notification_channels_query(&self.pool, encryption_key.as_ref())
            .await
    }

    async fn get_channel(&self, id: &str) -> AppResult<Option<NotificationChannelConfig>> {
        let encryption_key = self.encryption_key();
        notification_channel::get_notification_channel_query(
            &self.pool,
            id,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn create_channel(
        &self,
        config: NotificationChannelConfig,
    ) -> AppResult<NotificationChannelConfig> {
        self.db.create_notification_channel(config).await
    }

    async fn update_channel(
        &self,
        config: NotificationChannelConfig,
    ) -> AppResult<NotificationChannelConfig> {
        self.db.update_notification_channel(config).await
    }

    async fn delete_channel(&self, id: &str) -> AppResult<()> {
        self.db.delete_notification_channel(id).await
    }
}

#[async_trait]
impl NotificationSubscriptionRepository for SqliteNotificationStore {
    async fn list_subscriptions(&self) -> AppResult<Vec<NotificationSubscription>> {
        notification_subscription::list_notification_subscriptions_query(&self.pool).await
    }

    async fn list_subscriptions_for_channel(
        &self,
        channel_id: &str,
    ) -> AppResult<Vec<NotificationSubscription>> {
        notification_subscription::list_notification_subscriptions_for_channel_query(
            &self.pool, channel_id,
        )
        .await
    }

    async fn list_subscriptions_for_event(
        &self,
        event_type: NotificationEventType,
    ) -> AppResult<Vec<NotificationSubscription>> {
        notification_subscription::list_notification_subscriptions_for_event_query(
            &self.pool, event_type,
        )
        .await
    }

    async fn create_subscription(
        &self,
        sub: NotificationSubscription,
    ) -> AppResult<NotificationSubscription> {
        self.db.create_notification_subscription(sub).await
    }

    async fn update_subscription(
        &self,
        sub: NotificationSubscription,
    ) -> AppResult<NotificationSubscription> {
        self.db.update_notification_subscription(sub).await
    }

    async fn delete_subscription(&self, id: &str) -> AppResult<()> {
        self.db.delete_notification_subscription(id).await
    }
}
