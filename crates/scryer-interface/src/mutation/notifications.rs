use async_graphql::{Context, Object, Result as GqlResult};
use scryer_application::{
    NotificationScopeIdUpdate, NotificationSubscriptionTargetCreate,
    NotificationSubscriptionTargetUpdate,
};

use crate::context::{app_from_ctx, require_config_step_up, to_gql_error};
use crate::mappers::{from_notification_channel, from_notification_subscription};
use crate::types::*;

#[derive(Default)]
pub(crate) struct NotificationMutations;

#[Object]
impl NotificationMutations {
    async fn create_notification_channel(
        &self,
        ctx: &Context<'_>,
        input: CreateNotificationChannelInput,
    ) -> GqlResult<NotificationChannelPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_step_up(ctx).await?;
        let channel = app
            .create_notification_channel_with_media_server_connection_id(
                &actor,
                input.name,
                input.channel_type,
                input.config_json,
                input.media_server_connection_id,
                input.is_enabled.unwrap_or(true),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_notification_channel(channel))
    }

    async fn update_notification_channel(
        &self,
        ctx: &Context<'_>,
        input: UpdateNotificationChannelInput,
    ) -> GqlResult<NotificationChannelPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_step_up(ctx).await?;
        let channel = app
            .update_notification_channel_with_media_server_connection_id(
                &actor,
                input.id,
                input.name,
                input.config_json,
                input.media_server_connection_id,
                input.is_enabled,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_notification_channel(channel))
    }

    async fn delete_notification_channel(&self, ctx: &Context<'_>, id: String) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_step_up(ctx).await?;
        app.delete_notification_channel(&actor, &id)
            .await
            .map_err(to_gql_error)
            .map(|_| true)
    }

    async fn test_notification_channel(&self, ctx: &Context<'_>, id: String) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_step_up(ctx).await?;
        app.test_notification_channel(&actor, &id)
            .await
            .map_err(to_gql_error)
            .map(|_| true)
    }

    async fn create_notification_subscription(
        &self,
        ctx: &Context<'_>,
        input: CreateNotificationSubscriptionInput,
    ) -> GqlResult<NotificationSubscriptionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_step_up(ctx).await?;
        let sub = app
            .create_notification_subscription_for_target(
                &actor,
                NotificationSubscriptionTargetCreate {
                    channel_id: input.channel_id,
                    target_kind: input.target_kind,
                    target_id: input.target_id,
                    event_type: input.event_type,
                    scope: input.scope,
                    scope_id: input.scope_id,
                    is_enabled: input.is_enabled.unwrap_or(true),
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_notification_subscription(sub))
    }

    async fn update_notification_subscription(
        &self,
        ctx: &Context<'_>,
        input: UpdateNotificationSubscriptionInput,
    ) -> GqlResult<NotificationSubscriptionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_step_up(ctx).await?;
        let sub = app
            .update_notification_subscription_target(
                &actor,
                NotificationSubscriptionTargetUpdate {
                    id: input.id,
                    target_kind: input.target_kind,
                    target_id: input.target_id,
                    event_type: input.event_type,
                    scope: input.scope,
                    scope_id: input
                        .scope_id
                        .map(NotificationScopeIdUpdate::Set)
                        .unwrap_or(NotificationScopeIdUpdate::NoChange),
                    is_enabled: input.is_enabled,
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_notification_subscription(sub))
    }

    async fn delete_notification_subscription(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_step_up(ctx).await?;
        app.delete_notification_subscription(&actor, &id)
            .await
            .map_err(to_gql_error)
            .map(|_| true)
    }
}
