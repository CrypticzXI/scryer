use async_graphql::{Context, Object, Result as GqlResult};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{
    from_manual_plugin_preview, from_plugin_install_progress, from_plugin_installation,
    from_registry_plugin,
};
use crate::types::*;

#[derive(Default)]
pub(crate) struct PluginMutations;

#[Object]
impl PluginMutations {
    async fn refresh_plugin_registry(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<RegistryPluginPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let plugins = app
            .refresh_plugin_catalog(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(plugins.into_iter().map(from_registry_plugin).collect())
    }

    async fn refresh_plugin_catalog(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<RegistryPluginPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let plugins = app
            .refresh_plugin_catalog(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(plugins.into_iter().map(from_registry_plugin).collect())
    }

    async fn install_plugin(
        &self,
        ctx: &Context<'_>,
        input: InstallPluginInput,
    ) -> GqlResult<PluginInstallationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let installation = app
            .install_plugin(&actor, &input.plugin_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_installation(installation))
    }

    async fn begin_install_plugin(
        &self,
        ctx: &Context<'_>,
        input: InstallPluginInput,
    ) -> GqlResult<PluginInstallProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let snapshot = app
            .begin_install_plugin(&actor, &input.plugin_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_install_progress(snapshot))
    }

    async fn uninstall_plugin(
        &self,
        ctx: &Context<'_>,
        input: UninstallPluginInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.uninstall_plugin(&actor, &input.plugin_id)
            .await
            .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn toggle_plugin(
        &self,
        ctx: &Context<'_>,
        input: TogglePluginInput,
    ) -> GqlResult<PluginInstallationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let installation = app
            .toggle_plugin(&actor, &input.plugin_id, input.enabled)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_installation(installation))
    }

    async fn upgrade_plugin(
        &self,
        ctx: &Context<'_>,
        input: UpgradePluginInput,
    ) -> GqlResult<PluginInstallationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let installation = app
            .upgrade_plugin(&actor, &input.plugin_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_installation(installation))
    }

    async fn begin_upgrade_plugin(
        &self,
        ctx: &Context<'_>,
        input: UpgradePluginInput,
    ) -> GqlResult<PluginInstallProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let snapshot = app
            .begin_upgrade_plugin(&actor, &input.plugin_id)
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
        let actor = actor_from_ctx(ctx)?;
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
        let actor = actor_from_ctx(ctx)?;
        let installation = app
            .install_manual_plugin(&actor, &input.github_repo_url)
            .await
            .map_err(to_gql_error)?;
        Ok(from_plugin_installation(installation))
    }
}
