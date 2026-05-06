use async_graphql::{Context, Error, MaybeUndefined, Object, Result as GqlResult};
use chrono::{DateTime, Utc};
use scryer_application::{
    DownloadClientConfigUpdate, IndexerConfigUpdate, SubtitleProviderConfigUpdate,
};
use scryer_domain::{NewDownloadClientConfig, NewIndexerConfig};
use serde_json::{Value, json};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{
    from_download_client_config, from_housekeeping_report, from_indexer_config,
    from_rss_sync_report, from_subtitle_provider_config,
};
use crate::types::*;

fn should_seed_download_client_routing(client_type: &str) -> bool {
    matches!(client_type, "nzbget" | "sabnzbd" | "weaver")
}

fn optional_datetime_input(
    value: MaybeUndefined<String>,
    field_name: &str,
) -> GqlResult<Option<Option<DateTime<Utc>>>> {
    match value {
        MaybeUndefined::Undefined => Ok(None),
        MaybeUndefined::Null => Ok(Some(None)),
        MaybeUndefined::Value(value) => DateTime::parse_from_rfc3339(&value)
            .map(|value| Some(Some(value.with_timezone(&Utc))))
            .map_err(|error| Error::new(format!("invalid {field_name}: {error}"))),
    }
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
        validate_test_flight_url(&base_url)?;

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

    async fn create_subtitle_provider_config(
        &self,
        ctx: &Context<'_>,
        input: CreateSubtitleProviderConfigInput,
    ) -> GqlResult<SubtitleProviderConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let config = app
            .create_subtitle_provider_config(
                &actor,
                input.name,
                input.provider_type,
                input.config_json,
                input.enabled_facets.map(|facets| {
                    facets
                        .into_iter()
                        .map(|facet| facet.as_scope_id().to_string())
                        .collect()
                }),
                input.is_enabled.unwrap_or(true),
            )
            .await
            .map_err(to_gql_error)?;
        let config_fields = app.subtitle_provider_config_fields(&config.provider_type);
        Ok(from_subtitle_provider_config(config, &config_fields))
    }

    async fn update_subtitle_provider_config(
        &self,
        ctx: &Context<'_>,
        input: UpdateSubtitleProviderConfigInput,
    ) -> GqlResult<SubtitleProviderConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let disabled_until = optional_datetime_input(input.disabled_until, "disabled_until")?;
        let config = app
            .update_subtitle_provider_config(
                &actor,
                SubtitleProviderConfigUpdate {
                    id: input.id,
                    name: input.name,
                    provider_type: input.provider_type,
                    config_json: input.config_json,
                    enabled_facets: input.enabled_facets.map(|facets| {
                        facets
                            .into_iter()
                            .map(|facet| facet.as_scope_id().to_string())
                            .collect()
                    }),
                    is_enabled: input.is_enabled,
                    last_health_status: None,
                    last_error: None,
                    last_error_at: None,
                    disabled_until,
                },
            )
            .await
            .map_err(to_gql_error)?;
        let config_fields = app.subtitle_provider_config_fields(&config.provider_type);
        Ok(from_subtitle_provider_config(config, &config_fields))
    }

    async fn delete_subtitle_provider_config(
        &self,
        ctx: &Context<'_>,
        input: DeleteSubtitleProviderConfigInput,
    ) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.delete_subtitle_provider_config(&actor, &input.id)
            .await
            .map_err(to_gql_error)
            .map(|_| true)
    }

    async fn test_subtitle_provider_connection(
        &self,
        ctx: &Context<'_>,
        input: TestSubtitleProviderConnectionInput,
    ) -> GqlResult<SubtitleProviderValidationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let result = app
            .test_subtitle_provider_connection(
                &actor,
                input.id.as_deref(),
                input.provider_type,
                input.config_json,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(SubtitleProviderValidationPayload {
            status: result.status,
            message: result.message,
            retry_after_seconds: result.retry_after_seconds,
        })
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
        let report = app.run_housekeeping(&actor).await.map_err(to_gql_error)?;
        Ok(from_housekeeping_report(report))
    }

    async fn trigger_rss_sync(&self, ctx: &Context<'_>) -> GqlResult<RssSyncReportPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let report = app.run_rss_sync(&actor).await.map_err(to_gql_error)?;
        Ok(from_rss_sync_report(report))
    }
}

fn validate_test_flight_url(raw: &str) -> GqlResult<()> {
    let url = url::Url::parse(raw).map_err(|error| Error::new(format!("invalid URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::new("URL must use http or https"));
    }
    if url.host_str().is_none() {
        return Err(Error::new("URL must include a host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::new("URL must not include embedded credentials"));
    }
    Ok(())
}
