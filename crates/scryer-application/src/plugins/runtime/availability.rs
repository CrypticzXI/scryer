impl AppUseCase {
    /// Returns all available indexer provider types with their config field schemas.
    /// Tuple: (provider_type, name, config_fields, default_base_url)
    pub fn available_indexer_provider_types(
        &self,
    ) -> Vec<(
        String,
        String,
        Vec<scryer_domain::ConfigFieldDef>,
        Option<String>,
    )> {
        let Some(provider) = self.services.integrations.plugin_provider.available() else {
            return vec![];
        };
        let mut seen = std::collections::HashSet::new();
        provider
            .available_provider_types()
            .into_iter()
            .filter(|pt| seen.insert(pt.clone()))
            .map(|pt| {
                let name = provider
                    .plugin_name_for_provider(&pt)
                    .unwrap_or_else(|| pt.clone());
                let fields = provider.config_fields_for_provider(&pt);
                let default_base_url = provider.default_base_url_for_provider(&pt);
                (pt, name, fields, default_base_url)
            })
            .collect()
    }
}
impl AppUseCase {
    pub fn available_download_client_provider_types(
        &self,
    ) -> Vec<(
        String,
        String,
        Vec<scryer_domain::ConfigFieldDef>,
        Option<String>,
    )> {
        let Some(provider) = self
            .services
            .integrations
            .download_client_plugin_provider
            .available()
        else {
            return vec![];
        };
        let mut seen = std::collections::HashSet::new();
        provider
            .available_provider_types()
            .into_iter()
            .filter(|pt| seen.insert(pt.clone()))
            .map(|pt| {
                let name = provider
                    .plugin_name_for_provider(&pt)
                    .unwrap_or_else(|| pt.clone());
                let fields = provider.config_fields_for_provider(&pt);
                let default_base_url = provider.default_base_url_for_provider(&pt);
                (pt, name, fields, default_base_url)
            })
            .collect()
    }
}
impl AppUseCase {
    pub async fn test_download_client_connection(
        &self,
        actor: &User,
        client_type: &str,
        config_json: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let client_type = client_type.trim().to_lowercase();
        if matches!(client_type.as_str(), "nzbget" | "sabnzbd" | "weaver") {
            return self
                .services
                .integrations
                .builtin_download_client_connection_tester
                .test_connection(&client_type, config_json)
                .await;
        }

        self.test_plugin_download_client_connection(actor, &client_type, config_json)
            .await
    }
}
impl AppUseCase {
    pub async fn test_plugin_download_client_connection(
        &self,
        actor: &User,
        client_type: &str,
        config_json: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let provider = self
            .services
            .integrations
            .download_client_plugin_provider
            .available()
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "test connection is not supported for client type '{}'",
                    client_type.trim()
                ))
            })?;

        let client_type = client_type.trim().to_lowercase();
        let config_json = config_json.trim();
        let config_json = if config_json.is_empty() {
            "{}".to_string()
        } else {
            serde_json::to_string(
                &serde_json::from_str::<serde_json::Value>(config_json).map_err(|error| {
                    AppError::Validation(format!("invalid client config_json: {error}"))
                })?,
            )
            .map_err(|error| AppError::Validation(format!("invalid client config_json: {error}")))?
        };

        let now = chrono::Utc::now();
        let config = DownloadClientConfig {
            id: "test-download-client".to_string(),
            name: "Test Download Client".to_string(),
            client_type: client_type.clone(),
            config_json,
            client_priority: 0,
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            created_at: now,
            updated_at: now,
        };
        let client = provider.client_for_config(&config).ok_or_else(|| {
            AppError::Validation(format!(
                "test connection is not supported for client type '{client_type}'"
            ))
        })?;
        client.test_connection().await?;
        Ok(())
    }
}
fn indexer_config_can_be_auto_created(fields: &[scryer_domain::ConfigFieldDef]) -> bool {
    !fields.iter().any(|field| {
        field.required
            && field.value_source == scryer_domain::ConfigFieldValueSource::User
            && field.host_binding.is_none()
            && field.role != Some(scryer_domain::ConfigFieldRole::ConnectionUrl)
            && field
                .default_value
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
    })
}
