use super::*;
use chrono::Utc;

impl AppUseCase {
    /// Test an indexer connection by performing a minimal search through the plugin system.
    /// This validates: plugin availability, HTTP connectivity, API key, response parsing.
    pub async fn test_indexer_connection(
        &self,
        actor: &User,
        provider_type: &str,
        config_json: Option<&str>,
        indexer_id: Option<&str>,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let fields = self.indexer_config_fields_for_provider_type(provider_type)?;
        let persisted_config_json = if let Some(indexer_id) = indexer_id {
            self.services
                .integrations
                .indexer_configs
                .get_by_id(indexer_id)
                .await?
                .and_then(|config| config.config_json)
        } else {
            None
        };
        let normalized_config_json = crate::app_usecase_integration::normalize_indexer_config_json(
            &fields,
            config_json,
            persisted_config_json.as_deref(),
        )?;
        let base_url = crate::app_usecase_integration::derive_indexer_base_url_from_config_fields(
            &fields,
            Some(&normalized_config_json),
        )?;
        validate_test_flight_url(&base_url)?;

        let provider = self
            .services
            .integrations
            .plugin_provider
            .available()
            .ok_or_else(|| AppError::Repository("plugin provider not available".into()))?;

        let now = Utc::now();

        // Build a temporary IndexerConfig to get a client from the plugin
        // Reject obviously invalid API keys (e.g. masked placeholders from
        // Sonarr/Radarr import that were stored before the masking fix).
        let parsed_config: serde_json::Value = serde_json::from_str(&normalized_config_json)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        for field in fields
            .iter()
            .filter(|field| field.field_type == scryer_domain::ConfigFieldType::Password)
        {
            if let Some(trimmed) = parsed_config
                .get(&field.key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                && trimmed.chars().all(|c| c == '*')
                && !trimmed.is_empty()
            {
                return Err(AppError::Validation(
                    "API key appears to be a masked placeholder — enter the real key".into(),
                ));
            }
        }

        let temp_config = IndexerConfig {
            id: "test-connection".to_string(),
            name: "Test Connection".to_string(),
            provider_type: provider_type.to_string(),
            base_url,
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            disabled_until: None,
            last_health_status: None,
            last_error_at: None,
            config_json: Some(normalized_config_json),
            created_at: now,
            updated_at: now,
        };

        let client = provider.client_for_provider(&temp_config).ok_or_else(|| {
            AppError::Validation(format!(
                "no indexer plugin available for provider type '{provider_type}'"
            ))
        })?;

        // Perform a minimal search to validate the full pipeline
        client
            .search(
                String::new(), // empty query
                std::collections::HashMap::new(),
                None,
                None,
                None,
                None,
                SearchMode::Interactive,
                None,
                None,
                None,
                vec![],
            )
            .await
            .map_err(|e| AppError::Repository(format!("indexer connection test failed: {e}")))?;

        Ok(())
    }
}

fn validate_test_flight_url(raw: &str) -> AppResult<()> {
    let url = url::Url::parse(raw)
        .map_err(|error| AppError::Validation(format!("invalid base URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::Validation(
            "base URL must use http or https".into(),
        ));
    }
    if url.host_str().is_none() {
        return Err(AppError::Validation("base URL must include a host".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Validation(
            "base URL must not include embedded credentials".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NullSettingsRepository;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
        NullQualityProfileRepository, NullReleaseAttemptRepository, NullShowRepository,
        NullTitleRepository, NullUserRepository,
    };
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    struct RecordingIndexerConfigRepo {
        created: Arc<Mutex<Vec<IndexerConfig>>>,
    }

    impl RecordingIndexerConfigRepo {
        fn new() -> Self {
            Self {
                created: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl IndexerConfigRepository for RecordingIndexerConfigRepo {
        async fn list(&self, _provider_filter: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(self.created.lock().await.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
            let created = self.created.lock().await;
            Ok(created.iter().find(|config| config.id == id).cloned())
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            self.created.lock().await.push(config.clone());
            Ok(config)
        }

        async fn update(&self, _update: crate::IndexerConfigUpdate) -> AppResult<IndexerConfig> {
            Err(AppError::Repository("not implemented".into()))
        }

        async fn delete(&self, id: &str) -> AppResult<()> {
            self.created.lock().await.retain(|config| config.id != id);
            Ok(())
        }

        async fn touch_last_error(&self, _provider_type: &str) -> AppResult<()> {
            Ok(())
        }
    }

    struct RecordingPluginProvider {
        seen_configs: Arc<std::sync::Mutex<Vec<IndexerConfig>>>,
    }

    impl RecordingPluginProvider {
        fn new() -> Self {
            Self {
                seen_configs: Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }
    }

    impl IndexerPluginProvider for RecordingPluginProvider {
        fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
            self.seen_configs.lock().unwrap().push(config.clone());
            Some(Arc::new(NullIndexerClient))
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["torrent_rss".to_string()]
        }

        fn config_fields_for_provider(
            &self,
            provider_type: &str,
        ) -> Vec<scryer_domain::ConfigFieldDef> {
            if provider_type != "torrent_rss" {
                return vec![];
            }
            vec![scryer_domain::ConfigFieldDef {
                key: "feed_url".to_string(),
                label: "Feed URL".to_string(),
                field_type: scryer_domain::ConfigFieldType::String,
                required: true,
                default_value: None,
                value_source: scryer_domain::ConfigFieldValueSource::User,
                role: Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                host_binding: None,
                options: vec![],
                help_text: None,
            }]
        }

        fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
            vec![]
        }
    }

    fn test_app(
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        plugin_provider: Option<Arc<dyn IndexerPluginProvider>>,
    ) -> AppUseCase {
        let services = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            indexer_configs,
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        );
        let services = if let Some(plugin_provider) = plugin_provider {
            services
                .with_plugin_provider(plugin_provider)
                .build_partial_for_tests()
        } else {
            services.build_partial_for_tests()
        };

        AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    fn test_admin() -> User {
        let mut user = User::new_admin("admin");
        user.authorization = scryer_domain::UserAuthorization {
            app: scryer_domain::AppPermissionMask::from_permissions([
                scryer_domain::AppPermission::ManageSystemSettings,
                scryer_domain::AppPermission::ManageCatalogSettings,
            ]),
            loaded: true,
            ..Default::default()
        };
        user
    }

    #[tokio::test]
    async fn create_indexer_config_derives_base_url_from_feed_url() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let app = test_app(
            indexer_repo.clone(),
            Some(Arc::new(RecordingPluginProvider::new())),
        );

        let created = app
            .create_indexer_config(
                &test_admin(),
                NewIndexerConfig {
                    name: "RSS".to_string(),
                    provider_type: "torrent_rss".to_string(),
                    rate_limit_seconds: None,
                    rate_limit_burst: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    config_json: Some(
                        r#"{"feed_url":"https://ipt.beelyrics.net/t.rss?u=2203846"}"#.to_string(),
                    ),
                },
            )
            .await
            .unwrap();

        assert_eq!(created.base_url, "https://ipt.beelyrics.net");
    }

    #[tokio::test]
    async fn test_indexer_connection_derives_base_url_from_feed_url() {
        let provider = Arc::new(RecordingPluginProvider::new());
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider.clone()),
        );

        app.test_indexer_connection(
            &test_admin(),
            "torrent_rss",
            Some(r#"{"feed_url":"https://ipt.beelyrics.net/t.rss?u=2203846"}"#),
            None,
        )
        .await
        .unwrap();

        let seen = provider.seen_configs.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].base_url, "https://ipt.beelyrics.net");
    }
}
