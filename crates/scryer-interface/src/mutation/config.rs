use async_graphql::{Context, Error, Object, Result as GqlResult};
use scryer_application::{DownloadClientConfigUpdate, IndexerConfigUpdate};
use scryer_domain::{Entitlement, NewDownloadClientConfig, NewIndexerConfig};
use serde_json::{Value, json};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{
    from_download_client_config, from_housekeeping_report, from_indexer_config,
    from_rss_sync_report,
};
use crate::types::*;

fn should_seed_download_client_routing(client_type: &str) -> bool {
    matches!(client_type, "nzbget" | "sabnzbd" | "weaver")
}

#[derive(Default)]
pub(crate) struct ConfigMutations;

#[Object]
impl ConfigMutations {
    async fn create_indexer_config(
        &self,
        ctx: &Context<'_>,
        input: CreateIndexerConfigInput,
    ) -> GqlResult<IndexerConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let config = app
            .create_indexer_config(
                &actor,
                NewIndexerConfig {
                    name: input.name,
                    provider_type: input.provider_type,
                    base_url: input.base_url,
                    api_key_encrypted: input.api_key,
                    rate_limit_seconds: input.rate_limit_seconds,
                    rate_limit_burst: input.rate_limit_burst,
                    is_enabled: input.is_enabled.unwrap_or(true),
                    enable_interactive_search: input.enable_interactive_search.unwrap_or(true),
                    enable_auto_search: input.enable_auto_search.unwrap_or(true),
                    config_json: input.config_json,
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_indexer_config(config))
    }

    async fn update_indexer_config(
        &self,
        ctx: &Context<'_>,
        input: UpdateIndexerConfigInput,
    ) -> GqlResult<IndexerConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let config = app
            .update_indexer_config(
                &actor,
                IndexerConfigUpdate {
                    id: input.id,
                    name: input.name,
                    provider_type: input.provider_type,
                    base_url: input.base_url,
                    api_key_encrypted: input.api_key,
                    rate_limit_seconds: input.rate_limit_seconds,
                    rate_limit_burst: input.rate_limit_burst,
                    is_enabled: input.is_enabled,
                    enable_interactive_search: input.enable_interactive_search,
                    enable_auto_search: input.enable_auto_search,
                    config_json: input.config_json,
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_indexer_config(config))
    }

    async fn delete_indexer_config(
        &self,
        ctx: &Context<'_>,
        input: DeleteIndexerConfigInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_indexer_config(&actor, &input.id)
            .await
            .map_err(to_gql_error)
            .map(|_| true)
    }

    async fn create_download_client_config(
        &self,
        ctx: &Context<'_>,
        input: CreateDownloadClientConfigInput,
    ) -> GqlResult<DownloadClientConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let config = app
            .create_download_client_config(
                &actor,
                NewDownloadClientConfig {
                    name: input.name,
                    client_type: input.client_type,
                    config_json: input.config_json,
                    client_priority: 0,
                    is_enabled: input.is_enabled.unwrap_or(true),
                },
            )
            .await
            .map_err(to_gql_error)?;

        if should_seed_download_client_routing(&config.client_type) {
            app.ensure_download_client_routing_entry_for_client(&actor, &config.id)
                .await
                .map_err(to_gql_error)?;
        }

        Ok(from_download_client_config(config))
    }

    async fn update_download_client_config(
        &self,
        ctx: &Context<'_>,
        input: UpdateDownloadClientConfigInput,
    ) -> GqlResult<DownloadClientConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let config = app
            .update_download_client_config(
                &actor,
                DownloadClientConfigUpdate {
                    id: input.id,
                    name: input.name,
                    client_type: input.client_type,
                    config_json: input.config_json,
                    is_enabled: input.is_enabled,
                },
            )
            .await
            .map_err(to_gql_error)?;

        if should_seed_download_client_routing(&config.client_type) {
            app.ensure_download_client_routing_entry_for_client(&actor, &config.id)
                .await
                .map_err(to_gql_error)?;
        }

        Ok(from_download_client_config(config))
    }

    async fn delete_download_client_config(
        &self,
        ctx: &Context<'_>,
        input: DeleteDownloadClientConfigInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_download_client_config(&actor, &input.id)
            .await
            .map_err(to_gql_error)
            .map(|_| true)
    }

    async fn reorder_download_client_configs(
        &self,
        ctx: &Context<'_>,
        input: ReorderDownloadClientConfigsInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.reorder_download_clients(&actor, input.ids)
            .await
            .map_err(to_gql_error)
            .map(|_| true)
    }

    async fn test_download_client_connection(
        &self,
        ctx: &Context<'_>,
        input: TestDownloadClientConnectionInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }

        let client_type = input.client_type.trim().to_lowercase();

        let config_json = input.config_json.trim().to_string();
        let config: Value = if config_json.is_empty() {
            json!({})
        } else {
            serde_json::from_str(&config_json)
                .map_err(|error| Error::new(format!("invalid client config_json: {error}")))?
        };

        let base_url = scryer_infrastructure::resolve_base_url_from_config_json(&config_json)
            .ok_or_else(|| Error::new("cannot compute base URL from config — host is required"))?;

        match client_type.as_str() {
            "nzbget" => {
                let username = config
                    .get("username")
                    .and_then(Value::as_str)
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());
                let password = config
                    .get("password")
                    .and_then(Value::as_str)
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());

                scryer_infrastructure::NzbgetDownloadClient::new(
                    base_url,
                    username,
                    password,
                    "SCORE".to_string(),
                )
                .test_connection()
                .await
                .map_err(to_gql_error)?;
            }
            "sabnzbd" => {
                let api_key = config
                    .get("api_key")
                    .or_else(|| config.get("apiKey"))
                    .or_else(|| config.get("apikey"))
                    .and_then(Value::as_str)
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| Error::new("sabnzbd requires an API key"))?;

                scryer_infrastructure::SabnzbdDownloadClient::new(base_url, api_key)
                    .test_connection()
                    .await
                    .map_err(to_gql_error)?;
            }
            "weaver" => {
                let api_key = config
                    .get("api_key")
                    .or_else(|| config.get("apiKey"))
                    .or_else(|| config.get("apikey"))
                    .and_then(Value::as_str)
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());

                scryer_infrastructure::WeaverDownloadClient::new(base_url, api_key)
                    .test_connection()
                    .await
                    .map_err(to_gql_error)?;
            }
            _ => app
                .test_plugin_download_client_connection(&actor, &client_type, &config_json)
                .await
                .map_err(to_gql_error)?,
        }

        Ok(true)
    }

    async fn test_indexer_connection(
        &self,
        ctx: &Context<'_>,
        input: TestIndexerConnectionInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        // If no API key provided but an existing indexer ID is given,
        // look up the stored key so "Test Connection" works without
        // re-entering the key.
        let mut api_key = input.api_key;
        if api_key.as_ref().is_none_or(|k| k.trim().is_empty())
            && let Some(ref indexer_id) = input.indexer_id
            && let Ok(Some(config)) = app.get_indexer_config(&actor, indexer_id).await
            && let Some(ref stored_key) = config.api_key_encrypted
            && !stored_key.is_empty()
        {
            api_key = Some(stored_key.clone());
        }

        app.test_indexer_connection(
            &actor,
            &input.provider_type,
            &input.base_url,
            api_key.as_deref(),
            input.config_json.as_deref(),
        )
        .await
        .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn run_housekeeping(&self, ctx: &Context<'_>) -> GqlResult<HousekeepingReportPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let report = app.run_housekeeping().await.map_err(to_gql_error)?;
        Ok(from_housekeeping_report(report))
    }

    async fn trigger_rss_sync(&self, ctx: &Context<'_>) -> GqlResult<RssSyncReportPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let report = app.run_rss_sync().await.map_err(to_gql_error)?;
        Ok(from_rss_sync_report(report))
    }
}
