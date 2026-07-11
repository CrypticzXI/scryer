use async_graphql::{Context, ID, Object, Result as GqlResult};
use scryer_domain::AppPermission;

use crate::context::{app_from_ctx, require_config_app_permission, to_gql_error};
use crate::mappers::{
    from_manual_plugin_preview, from_plugin_install_progress, from_plugin_installation,
    from_registry_plugin,
};
use crate::types::*;

#[derive(Default)]
pub(crate) struct PluginMutations;

#[Object]
impl PluginMutations {
    async fn refresh_plugin_catalog(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<RegistryPluginPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let plugins = app
            .refresh_plugin_catalog(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(plugins.into_iter().map(from_registry_plugin).collect())
    }

    async fn begin_install_plugin(
        &self,
        ctx: &Context<'_>,
        plugin_id: ID,
    ) -> GqlResult<PluginInstallProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let plugin_id = String::from(plugin_id);
        let snapshot = app
            .begin_install_plugin(&actor, &plugin_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_install_progress(snapshot))
    }

    async fn uninstall_plugin(
        &self,
        ctx: &Context<'_>,
        plugin_id: ID,
    ) -> GqlResult<UninstallPluginPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let plugin_id = String::from(plugin_id);
        app.uninstall_plugin(&actor, &plugin_id)
            .await
            .map_err(to_gql_error)?;
        Ok(UninstallPluginPayload {
            plugin_id: ID::from(plugin_id),
        })
    }

    async fn toggle_plugin(
        &self,
        ctx: &Context<'_>,
        input: TogglePluginInput,
    ) -> GqlResult<PluginInstallationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let installation = app
            .toggle_plugin(&actor, input.plugin_id.as_ref(), input.enabled)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_installation(installation))
    }

    async fn begin_upgrade_plugin(
        &self,
        ctx: &Context<'_>,
        plugin_id: ID,
    ) -> GqlResult<PluginInstallProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let plugin_id = String::from(plugin_id);
        let snapshot = app
            .begin_upgrade_plugin(&actor, &plugin_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_install_progress(snapshot))
    }

    async fn inspect_manual_plugin_repo(
        &self,
        ctx: &Context<'_>,
        input: ManualPluginRepoInput,
    ) -> GqlResult<ManualPluginPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let preview = app
            .inspect_manual_plugin_repo(&actor, &input.github_repo_url)
            .await
            .map_err(to_gql_error)?;
        Ok(from_manual_plugin_preview(preview))
    }

    async fn install_manual_plugin(
        &self,
        ctx: &Context<'_>,
        input: ManualPluginRepoInput,
    ) -> GqlResult<PluginInstallationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let installation = app
            .install_manual_plugin(&actor, &input.github_repo_url)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_installation(installation))
    }

    async fn install_uploaded_plugin(
        &self,
        ctx: &Context<'_>,
        input: ManualPluginUploadInput,
    ) -> GqlResult<PluginInstallationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let installation = app
            .install_uploaded_plugin(
                &actor,
                &input.file_name,
                &input.wasm_base64,
                input.acknowledge_risk,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_installation(installation))
    }
}
