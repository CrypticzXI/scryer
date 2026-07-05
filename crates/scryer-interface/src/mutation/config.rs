use async_graphql::{Context, ID, MaybeUndefined, Object, Result as GqlResult};
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, DownloadClientConfigUpdate, IndexerConfigUpdate, IndexerProxyConfigUpdate,
    NewIndexerProxyConfig, SubtitleProviderConfigUpdate,
};
use scryer_domain::{
    AppPermission, IndexerProxyProviderType, NewDownloadClientConfig, NewIndexerConfig,
};

use crate::context::{actor_from_ctx, app_from_ctx, require_config_app_permission, to_gql_error};
use crate::mappers::{
    from_download_client_config_with_fields, from_indexer_config_sync_result,
    from_indexer_config_with_fields, from_indexer_proxy_config, from_indexer_proxy_test_result,
    from_rss_sync_report, from_subtitle_provider_config, provider_config_values_to_json,
};
use crate::types::*;

fn should_seed_download_client_routing(client_type: &str) -> bool {
    matches!(client_type, "nzbget" | "sabnzbd" | "weaver")
}

fn optional_datetime_input(
    value: MaybeUndefined<DateTime<Utc>>,
    _field_name: &str,
) -> GqlResult<Option<Option<DateTime<Utc>>>> {
    match value {
        MaybeUndefined::Undefined => Ok(None),
        MaybeUndefined::Null => Ok(Some(None)),
        MaybeUndefined::Value(value) => Ok(Some(Some(value))),
    }
}

fn optional_id_input(value: MaybeUndefined<ID>) -> Option<Option<String>> {
    match value {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(value) => Some(Some(value.to_string())),
    }
}

async fn enrich_download_client_config_json(
    _client_type: &str,
    config_json: String,
) -> GqlResult<String> {
    Ok(config_json)
}

fn download_client_config_fields(
    app: &scryer_application::AppUseCase,
    client_type: &str,
) -> Vec<scryer_domain::ConfigFieldDef> {
    app.available_download_client_provider_types()
        .into_iter()
        .find_map(|(provider_type, _, fields, _)| {
            provider_type
                .eq_ignore_ascii_case(client_type)
                .then_some(fields)
        })
        .unwrap_or_default()
}

fn provider_config_key_looks_secret(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase();
    normalized.contains("password")
        || normalized.contains("secret")
        || normalized.contains("token")
        || normalized == "username"
        || normalized == "user_name"
        || normalized == "api_key"
        || normalized == "apikey"
        || normalized.contains("api_key")
}

fn merge_omitted_provider_secrets(
    incoming_json: String,
    existing_json: &str,
    config_fields: &[scryer_domain::ConfigFieldDef],
) -> scryer_application::AppResult<String> {
    let mut incoming = serde_json::from_str::<serde_json::Value>(&incoming_json)
        .map_err(|error| scryer_application::AppError::Validation(error.to_string()))?;
    let existing = serde_json::from_str::<serde_json::Value>(existing_json)
        .map_err(|error| scryer_application::AppError::Validation(error.to_string()))?;
    let Some(incoming_object) = incoming.as_object_mut() else {
        return Ok(incoming_json);
    };
    let Some(existing_object) = existing.as_object() else {
        return Ok(incoming_json);
    };
    let configured_secret_keys = config_fields
        .iter()
        .filter(|field| field.field_type == scryer_domain::ConfigFieldType::Password)
        .map(|field| field.key.as_str())
        .collect::<std::collections::HashSet<_>>();

    for (key, value) in existing_object {
        let is_secret =
            configured_secret_keys.contains(key.as_str()) || provider_config_key_looks_secret(key);
        if is_secret && !incoming_object.contains_key(key) {
            incoming_object.insert(key.clone(), value.clone());
        }
    }

    serde_json::to_string(&incoming)
        .map_err(|error| scryer_application::AppError::Validation(error.to_string()))
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
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let config_json = input
            .config
            .map(provider_config_values_to_json)
            .transpose()
            .map_err(to_gql_error)?;
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
                    indexer_proxy_config_id: input.indexer_proxy_config_id.map(|id| id.to_string()),
                    config_json,
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
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let config_json = input
            .config
            .map(provider_config_values_to_json)
            .transpose()
            .map_err(to_gql_error)?;
        let config = app
            .update_indexer_config(
                &actor,
                IndexerConfigUpdate {
                    id: input.id.to_string(),
                    name: input.name,
                    provider_type: input.provider_type,
                    derived_base_url: None,
                    rate_limit_seconds: input.rate_limit_seconds,
                    rate_limit_burst: input.rate_limit_burst,
                    is_enabled: input.is_enabled,
                    enable_interactive_search: input.enable_interactive_search,
                    enable_auto_search: input.enable_auto_search,
                    indexer_proxy_config_id: optional_id_input(input.indexer_proxy_config_id),
                    managed_parent_config_id: None,
                    managed_child_key: None,
                    managed_metadata_json: None,
                    caps_snapshot_json: None,
                    config_json,
                },
            )
            .await
            .map_err(to_gql_error)?;
        let config_fields = app
            .indexer_config_fields_for_provider_type(&config.provider_type)
            .unwrap_or_default();
        Ok(from_indexer_config_with_fields(config, &config_fields))
    }

    async fn create_indexer_proxy_config(
        &self,
        ctx: &Context<'_>,
        input: CreateIndexerProxyConfigInput,
    ) -> GqlResult<IndexerProxyConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let provider_type =
            IndexerProxyProviderType::parse(&input.provider_type).ok_or_else(|| {
                to_gql_error(AppError::Validation(format!(
                    "unsupported indexer proxy provider '{}'",
                    input.provider_type
                )))
            })?;
        let request_timeout_seconds = input
            .request_timeout_seconds
            .map(|value| {
                u32::try_from(value).map_err(|_| {
                    to_gql_error(AppError::Validation(
                        "request timeout seconds must be positive".into(),
                    ))
                })
            })
            .transpose()?;
        let config = app
            .create_indexer_proxy_config(
                &actor,
                NewIndexerProxyConfig {
                    name: input.name,
                    provider_type,
                    base_url: input.base_url,
                    request_timeout_seconds,
                    is_enabled: input.is_enabled.unwrap_or(true),
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_indexer_proxy_config(config))
    }

    async fn update_indexer_proxy_config(
        &self,
        ctx: &Context<'_>,
        input: UpdateIndexerProxyConfigInput,
    ) -> GqlResult<IndexerProxyConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let request_timeout_seconds = input
            .request_timeout_seconds
            .map(|value| {
                u32::try_from(value).map_err(|_| {
                    to_gql_error(AppError::Validation(
                        "request timeout seconds must be positive".into(),
                    ))
                })
            })
            .transpose()?;
        let config = app
            .update_indexer_proxy_config(
                &actor,
                IndexerProxyConfigUpdate {
                    id: input.id.to_string(),
                    name: input.name,
                    base_url: input.base_url,
                    request_timeout_seconds,
                    is_enabled: input.is_enabled,
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_indexer_proxy_config(config))
    }

    async fn delete_indexer_proxy_config(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<DeleteIndexerProxyConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        app.delete_indexer_proxy_config(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteIndexerProxyConfigPayload { ok: true })
    }

    async fn test_indexer_proxy_config(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<IndexerProxyTestResultPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let result = app
            .test_indexer_proxy_config(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(from_indexer_proxy_test_result(result))
    }

    async fn delete_indexer_config(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<DeleteIndexerConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let id = id.to_string();
        app.delete_indexer_config(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteIndexerConfigPayload {
            id: ID::from(id),
            deleted: true,
        })
    }

    async fn create_download_client_config(
        &self,
        ctx: &Context<'_>,
        input: CreateDownloadClientConfigInput,
    ) -> GqlResult<DownloadClientConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let config_json = provider_config_values_to_json(input.config).map_err(to_gql_error)?;
        let config_json =
            enrich_download_client_config_json(&input.client_type, config_json).await?;
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

        let config_fields = download_client_config_fields(&app, &config.client_type);
        Ok(from_download_client_config_with_fields(
            config,
            &config_fields,
        ))
    }

    async fn update_download_client_config(
        &self,
        ctx: &Context<'_>,
        input: UpdateDownloadClientConfigInput,
    ) -> GqlResult<DownloadClientConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let existing = app
            .get_download_client_config(&actor, input.id.as_ref())
            .await
            .map_err(to_gql_error)?
            .ok_or_else(|| {
                to_gql_error(AppError::NotFound(format!(
                    "download client {}",
                    input.id.as_ref()
                )))
            })?;
        let effective_client_type = input
            .client_type
            .as_deref()
            .unwrap_or(existing.client_type.as_str())
            .to_string();
        let effective_config_json = match input.config {
            Some(config) => {
                let config_json = provider_config_values_to_json(config).map_err(to_gql_error)?;
                let config_fields = download_client_config_fields(&app, &effective_client_type);
                let config_json = merge_omitted_provider_secrets(
                    config_json,
                    &existing.config_json,
                    &config_fields,
                )
                .map_err(to_gql_error)?;
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
                    id: input.id.to_string(),
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

        let config_fields = download_client_config_fields(&app, &config.client_type);
        Ok(from_download_client_config_with_fields(
            config,
            &config_fields,
        ))
    }

    async fn delete_download_client_config(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<DeleteDownloadClientConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let id = id.to_string();
        app.delete_download_client_config(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteDownloadClientConfigPayload {
            id: ID::from(id),
            deleted: true,
        })
    }

    async fn reorder_download_client_configs(
        &self,
        ctx: &Context<'_>,
        input: ReorderDownloadClientConfigsInput,
    ) -> GqlResult<ReorderDownloadClientConfigsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let ids = input.ids;
        let id_strings = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        app.reorder_download_clients(&actor, id_strings)
            .await
            .map_err(to_gql_error)?;
        Ok(ReorderDownloadClientConfigsPayload {
            ids,
            reordered: true,
        })
    }

    async fn test_download_client_connection(
        &self,
        ctx: &Context<'_>,
        input: TestDownloadClientConnectionInput,
    ) -> GqlResult<ProviderValidationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;

        let client_type = input.client_type.trim().to_lowercase();
        let config_json = provider_config_values_to_json(input.config).map_err(to_gql_error)?;
        app.test_download_client_connection(&actor, &client_type, &config_json)
            .await
            .map_err(to_gql_error)?;
        Ok(ProviderValidationPayload {
            status: "ok".to_string(),
            message: None,
            retry_after_seconds: None,
        })
    }

    async fn create_subtitle_provider_config(
        &self,
        ctx: &Context<'_>,
        input: CreateSubtitleProviderConfigInput,
    ) -> GqlResult<SubtitleProviderConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let config_json = provider_config_values_to_json(input.config).map_err(to_gql_error)?;
        let config = app
            .create_subtitle_provider_config(
                &actor,
                input.name,
                input.provider_type,
                config_json,
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
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let disabled_until = optional_datetime_input(input.disabled_until, "disabled_until")?;
        let config_json = input
            .config
            .map(provider_config_values_to_json)
            .transpose()
            .map_err(to_gql_error)?;
        let config = app
            .update_subtitle_provider_config(
                &actor,
                SubtitleProviderConfigUpdate {
                    id: input.id.to_string(),
                    name: input.name,
                    provider_type: input.provider_type,
                    config_json,
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
        id: ID,
    ) -> GqlResult<DeleteSubtitleProviderConfigPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let id = id.to_string();
        app.delete_subtitle_provider_config(&actor, &id)
            .await
            .map_err(to_gql_error)?;
        Ok(DeleteSubtitleProviderConfigPayload {
            id: ID::from(id),
            deleted: true,
        })
    }

    async fn test_subtitle_provider_connection(
        &self,
        ctx: &Context<'_>,
        input: TestSubtitleProviderConnectionInput,
    ) -> GqlResult<ProviderValidationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let result = app
            .test_subtitle_provider_connection(
                &actor,
                input.id.as_ref().map(|id| id.as_ref()),
                input.provider_type,
                provider_config_values_to_json(input.config).map_err(to_gql_error)?,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(ProviderValidationPayload {
            status: result.status,
            message: result.message,
            retry_after_seconds: result.retry_after_seconds,
        })
    }

    async fn test_indexer_connection(
        &self,
        ctx: &Context<'_>,
        input: TestIndexerConnectionInput,
    ) -> GqlResult<ProviderValidationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let config_json = input
            .config
            .map(provider_config_values_to_json)
            .transpose()
            .map_err(to_gql_error)?;

        app.test_indexer_connection(
            &actor,
            &input.provider_type,
            config_json.as_deref(),
            input.indexer_id.as_ref().map(|id| id.as_ref()),
            match &input.indexer_proxy_config_id {
                async_graphql::MaybeUndefined::Undefined => None,
                async_graphql::MaybeUndefined::Null => Some(None),
                async_graphql::MaybeUndefined::Value(id) => Some(Some(id.as_ref())),
            },
        )
        .await
        .map_err(to_gql_error)?;
        Ok(ProviderValidationPayload {
            status: "ok".to_string(),
            message: None,
            retry_after_seconds: None,
        })
    }

    async fn sync_indexer_config(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> GqlResult<IndexerConfigSyncPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let id = id.to_string();
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
