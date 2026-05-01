use chrono::Utc;
use scryer_domain::{
    Id, NotificationChannelConfig, NotificationEventType, NotificationSubscription,
};

use crate::ports::NOTIFICATION_REQUEST_SCHEMA_VERSION;
use crate::{
    AppError, AppResult, AppUseCase, NotificationAppPayload, NotificationPayload,
    NotificationScopeIdUpdate,
};

fn parse_subscribable_notification_event_type(event_type: &str) -> AppResult<NotificationEventType> {
    let parsed = NotificationEventType::parse(event_type).ok_or_else(|| {
        AppError::Validation(format!("unknown notification event type: {event_type}"))
    })?;

    if crate::notifications::dispatcher::supported_notification_event_types()
        .iter()
        .any(|candidate| candidate == &parsed)
    {
        Ok(parsed)
    } else {
        Err(AppError::Validation(format!(
            "notification event type is not subscribable: {event_type}"
        )))
    }
}

impl AppUseCase {
    pub fn subscribable_notification_event_types(&self) -> &'static [NotificationEventType] {
        crate::notifications::dispatcher::supported_notification_event_types()
    }

    pub async fn list_notification_channels(
        &self,
        actor: &scryer_domain::User,
    ) -> AppResult<Vec<NotificationChannelConfig>> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;
        let repo = self.notification_channels()?;
        repo.list_channels().await
    }

    pub async fn get_notification_channel(
        &self,
        actor: &scryer_domain::User,
        id: &str,
    ) -> AppResult<Option<NotificationChannelConfig>> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;
        let repo = self.notification_channels()?;
        repo.get_channel(id).await
    }

    pub async fn create_notification_channel(
        &self,
        actor: &scryer_domain::User,
        name: String,
        channel_type: String,
        config_json: String,
        is_enabled: bool,
    ) -> AppResult<NotificationChannelConfig> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;

        if name.trim().is_empty() {
            return Err(AppError::Validation(
                "channel name must not be empty".into(),
            ));
        }
        let channel_type = scryer_domain::ChannelType::parse(channel_type.trim())
            .ok_or_else(|| AppError::Validation(format!("invalid channel type: {channel_type}")))?;

        let now = Utc::now();
        let config = NotificationChannelConfig {
            id: Id::new().0,
            name,
            channel_type,
            config_json,
            is_enabled,
            created_at: now,
            updated_at: now,
        };

        let repo = self.notification_channels()?;
        repo.create_channel(config).await
    }

    pub async fn update_notification_channel(
        &self,
        actor: &scryer_domain::User,
        id: String,
        name: Option<String>,
        config_json: Option<String>,
        is_enabled: Option<bool>,
    ) -> AppResult<NotificationChannelConfig> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;
        let repo = self.notification_channels()?;

        let mut channel = repo
            .get_channel(&id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("notification channel {id}")))?;

        if let Some(n) = name {
            channel.name = n;
        }
        if let Some(c) = config_json {
            channel.config_json = c;
        }
        if let Some(e) = is_enabled {
            channel.is_enabled = e;
        }
        channel.updated_at = Utc::now();

        repo.update_channel(channel).await
    }

    pub async fn delete_notification_channel(
        &self,
        actor: &scryer_domain::User,
        id: &str,
    ) -> AppResult<()> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;
        let repo = self.notification_channels()?;
        repo.delete_channel(id).await
    }

    pub async fn test_notification_channel(
        &self,
        actor: &scryer_domain::User,
        id: &str,
    ) -> AppResult<()> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;

        let repo = self.notification_channels()?;
        let channel = repo
            .get_channel(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("notification channel {id}")))?;

        let provider = self
            .services
            .notifications
            .notification_provider()
            .ok_or_else(|| {
                AppError::Repository("notification plugin provider is not configured".into())
            })?;

        let client = provider.client_for_channel(&channel).ok_or_else(|| {
            AppError::NotFound(format!(
                "no notification plugin for channel type '{}'",
                channel.channel_type.as_str()
            ))
        })?;

        client
            .send_notification(&NotificationPayload {
                schema_version: NOTIFICATION_REQUEST_SCHEMA_VERSION,
                event_type: NotificationEventType::Test,
                event_id: None,
                occurred_at: None,
                correlation_id: None,
                actor: None,
                severity: None,
                is_test: true,
                summary_title: "Scryer Test Notification".to_string(),
                summary_message: "This is a test notification from Scryer.".to_string(),
                app: NotificationAppPayload {
                    name: "Scryer".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                },
                title: None,
                episode: None,
                episodes: Vec::new(),
                release: None,
                download: None,
                import: None,
                health: None,
                file: None,
                media_files: Vec::new(),
                application_update: None,
                manual_interaction: None,
            })
            .await
    }

    pub async fn list_notification_subscriptions(
        &self,
        actor: &scryer_domain::User,
    ) -> AppResult<Vec<NotificationSubscription>> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;
        let repo = self.notification_subscriptions()?;
        repo.list_subscriptions().await
    }

    pub async fn create_notification_subscription(
        &self,
        actor: &scryer_domain::User,
        channel_id: String,
        event_type: String,
        scope: String,
        scope_id: Option<String>,
        is_enabled: bool,
    ) -> AppResult<NotificationSubscription> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;

        let parsed_event_type = parse_subscribable_notification_event_type(&event_type)?;

        // Validate channel exists
        let ch_repo = self.notification_channels()?;
        ch_repo
            .get_channel(&channel_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("notification channel {channel_id}")))?;

        let now = Utc::now();
        let sub = NotificationSubscription {
            id: Id::new().0,
            channel_id,
            event_type: parsed_event_type,
            scope,
            scope_id,
            is_enabled,
            created_at: now,
            updated_at: now,
        };

        let repo = self.notification_subscriptions()?;
        repo.create_subscription(sub).await
    }

    pub async fn update_notification_subscription(
        &self,
        actor: &scryer_domain::User,
        id: String,
        event_type: Option<String>,
        scope: Option<String>,
        scope_id: NotificationScopeIdUpdate,
        is_enabled: Option<bool>,
    ) -> AppResult<NotificationSubscription> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;
        let repo = self.notification_subscriptions()?;

        // Find all subscriptions and locate ours
        let all = repo.list_subscriptions().await?;
        let mut sub = all
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| AppError::NotFound(format!("notification subscription {id}")))?;

        if let Some(et) = event_type {
            let parsed = parse_subscribable_notification_event_type(&et)?;
            sub.event_type = parsed;
        }
        if let Some(s) = scope {
            sub.scope = s;
        }
        match scope_id {
            NotificationScopeIdUpdate::NoChange => {}
            NotificationScopeIdUpdate::Clear => sub.scope_id = None,
            NotificationScopeIdUpdate::Set(scope_id) => sub.scope_id = Some(scope_id),
        }
        if let Some(e) = is_enabled {
            sub.is_enabled = e;
        }
        sub.updated_at = Utc::now();

        repo.update_subscription(sub).await
    }

    pub async fn delete_notification_subscription(
        &self,
        actor: &scryer_domain::User,
        id: &str,
    ) -> AppResult<()> {
        crate::require(actor, &scryer_domain::Entitlement::ManageConfig)?;
        let repo = self.notification_subscriptions()?;
        repo.delete_subscription(id).await
    }

    pub fn available_notification_provider_types(&self) -> Vec<String> {
        self.services
            .notifications
            .notification_provider()
            .map(|p| p.available_provider_types())
            .unwrap_or_default()
    }

    pub fn notification_provider_config_fields(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        self.services
            .notifications
            .notification_provider()
            .map(|p| p.config_fields_for_provider(provider_type))
            .unwrap_or_default()
    }

    pub fn notification_provider_name(&self, provider_type: &str) -> Option<String> {
        self.services
            .notifications
            .notification_provider()
            .and_then(|p| p.plugin_name_for_provider(provider_type))
    }

    pub fn notification_channels_repo(
        &self,
    ) -> AppResult<&std::sync::Arc<dyn crate::NotificationChannelRepository>> {
        self.services
            .notifications
            .notification_channels()
            .ok_or_else(|| {
                AppError::Repository("notification channel repository is not configured".into())
            })
    }

    pub fn notification_subscriptions_repo(
        &self,
    ) -> AppResult<&std::sync::Arc<dyn crate::NotificationSubscriptionRepository>> {
        self.services
            .notifications
            .notification_subscriptions()
            .ok_or_else(|| {
                AppError::Repository(
                    "notification subscription repository is not configured".into(),
                )
            })
    }

    // Helper to get notification channel repository
    fn notification_channels(
        &self,
    ) -> AppResult<&std::sync::Arc<dyn crate::NotificationChannelRepository>> {
        self.notification_channels_repo()
    }

    // Helper to get notification subscription repository
    fn notification_subscriptions(
        &self,
    ) -> AppResult<&std::sync::Arc<dyn crate::NotificationSubscriptionRepository>> {
        self.notification_subscriptions_repo()
    }
}
