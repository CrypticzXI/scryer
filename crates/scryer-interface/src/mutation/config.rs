use async_graphql::{Context, Error, MaybeUndefined, Object, Result as GqlResult};
use chrono::{DateTime, Utc};
use scryer_application::{
    DownloadClientConfigUpdate, IndexerConfigUpdate, SubtitleProviderConfigUpdate,
};
use scryer_domain::{NewDownloadClientConfig, NewIndexerConfig};

use crate::context::{actor_from_ctx, app_from_ctx, to_gql_error};
use crate::mappers::{
    from_download_client_config, from_indexer_config_sync_result, from_indexer_config_with_fields,
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

async fn enrich_download_client_config_json(
    _client_type: &str,
    config_json: String,
) -> GqlResult<String> {
    Ok(config_json)
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
        let config_fields = app
            .indexer_config_fields_for_provider_type(&config.provider_type)
            .unwrap_or_default();

        Ok(from_indexer_config_with_fields(config, &config_fields))
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
                    derived_base_url: None,
                    rate_limit_seconds: input.rate_limit_seconds,
                    rate_limit_burst: input.rate_limit_burst,
                    is_enabled: input.is_enabled,
                    enable_interactive_search: input.enable_interactive_search,
                    enable_auto_search: input.enable_auto_search,
                    managed_parent_config_id: None,
                    managed_child_key: None,
                    managed_metadata_json: None,
                    caps_snapshot_json: None,
                    config_json: input.config_json,
                },
            )
            .await
            .map_err(to_gql_error)?;
        let config_fields = app
            .indexer_config_fields_for_provider_type(&config.provider_type)
            .unwrap_or_default();
        Ok(from_indexer_config_with_fields(config, &config_fields))
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
        let config_json =
            enrich_download_client_config_json(&input.client_type, input.config_json).await?;
        let config = app
            .create_download_client_config(
                &actor,
                NewDownloadClientConfig {
                    name: input.name,
                    client_type: input.client_type,
                    config_json,
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
        let existing = app
            .get_download_client_config(&actor, &input.id)
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| Error::new(format!("download client not found: {}", input.id)))?;
        let effective_client_type = input
            .client_type
            .as_deref()
            .unwrap_or(existing.client_type.as_str())
            .to_string();
        let effective_config_json = match input.config_json {
            Some(config_json) => {
                Some(enrich_download_client_config_json(&effective_client_type, config_json).await?)
            }
            None if input
                .client_type
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case("sabnzbd")) =>
            {
                Some(
                    enrich_download_client_config_json(
                        &effective_client_type,
                        existing.config_json.clone(),
                    )
                    .await?,
                )
            }
            None => None,
        };
        let config = app
            .update_download_client_config(
                &actor,
                DownloadClientConfigUpdate {
                    id: input.id,
                    name: input.name,
                    client_type: input.client_type,
                    config_json: effective_config_json,
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
        app.test_download_client_connection(&actor, &client_type, &config_json)
            .await
            .map_err(to_gql_error)?;
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

        app.test_indexer_connection(
            &actor,
            &input.provider_type,
            input.config_json.as_deref(),
            input.indexer_id.as_deref(),
        )
        .await
        .map_err(to_gql_error)?;
        Ok(true)
    }

    async fn sync_indexer_config(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<IndexerConfigSyncPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let result = app
            .sync_indexer_config(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_indexer_config_sync_result(result))
    }

    async fn trigger_rss_sync(&self, ctx: &Context<'_>) -> GqlResult<RssSyncReportPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let report = app.run_rss_sync(&actor).await.map_err(to_gql_error)?;
        Ok(from_rss_sync_report(report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enrich_download_client_config_json_leaves_sab_config_unchanged() {
        let config_json = enrich_download_client_config_json(
            "sabnzbd",
            r#"{"host":"127.0.0.1","port":"8080","use_ssl":false,"api_key":"test-api-key"}"#
                .to_string(),
        )
        .await
        .expect("config enrichment should succeed");

        assert_eq!(
            config_json,
            r#"{"host":"127.0.0.1","port":"8080","use_ssl":false,"api_key":"test-api-key"}"#
        );
    }

    #[tokio::test]
    async fn enrich_download_client_config_json_leaves_other_client_config_unchanged() {
        let config_json = enrich_download_client_config_json(
            "weaver",
            r#"{"host":"127.0.0.1","port":"8081"}"#.to_string(),
        )
        .await
        .expect("config enrichment should succeed");

        assert_eq!(config_json, r#"{"host":"127.0.0.1","port":"8081"}"#);
    }
}
