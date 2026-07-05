impl AppUseCase {
    pub async fn list_indexer_proxy_configs(
        &self,
        actor: &User,
    ) -> AppResult<Vec<scryer_domain::IndexerProxyConfig>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.services.integrations.indexer_proxy_configs.list(None).await
    }

    pub async fn create_indexer_proxy_config(
        &self,
        actor: &User,
        input: NewIndexerProxyConfig,
    ) -> AppResult<scryer_domain::IndexerProxyConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let name = normalize_indexer_proxy_name(&input.name)?;
        let base_url = normalize_indexer_proxy_base_url(&input.base_url)?;
        let request_timeout_seconds =
            validate_indexer_proxy_timeout(input.request_timeout_seconds.unwrap_or(60))?;
        let now = Utc::now();
        let config = scryer_domain::IndexerProxyConfig {
            id: Id::new().0,
            name,
            provider_type: input.provider_type,
            protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
            base_url,
            request_timeout_seconds,
            is_enabled: input.is_enabled,
            last_health_status: Some(scryer_domain::IndexerProxyHealthStatus::Unknown),
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        };
        self.services
            .integrations
            .indexer_proxy_configs
            .create(config)
            .await
    }

    pub async fn update_indexer_proxy_config(
        &self,
        actor: &User,
        update: IndexerProxyConfigUpdate,
    ) -> AppResult<scryer_domain::IndexerProxyConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let id = update.id.trim();
        if id.is_empty() {
            return Err(AppError::Validation("indexer proxy config id is required".into()));
        }
        if !update.has_changes() {
            return Err(AppError::Validation(
                "at least one indexer proxy field must be provided".into(),
            ));
        }

        let mut config = self
            .services
            .integrations
            .indexer_proxy_configs
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("indexer proxy config '{id}' not found")))?;
        if let Some(name) = update.name {
            config.name = normalize_indexer_proxy_name(&name)?;
        }
        if let Some(base_url) = update.base_url {
            config.base_url = normalize_indexer_proxy_base_url(&base_url)?;
        }
        if let Some(timeout) = update.request_timeout_seconds {
            config.request_timeout_seconds = validate_indexer_proxy_timeout(timeout)?;
        }
        if let Some(is_enabled) = update.is_enabled {
            config.is_enabled = is_enabled;
        }
        config.updated_at = Utc::now();

        self.services
            .integrations
            .indexer_proxy_configs
            .update(config)
            .await
    }

    pub async fn delete_indexer_proxy_config(&self, actor: &User, id: &str) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let id = id.trim();
        if id.is_empty() {
            return Err(AppError::Validation("indexer proxy config id is required".into()));
        }
        let assigned = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?
            .into_iter()
            .any(|indexer| indexer.indexer_proxy_config_id.as_deref() == Some(id));
        if assigned {
            return Err(AppError::Validation(
                "indexer proxy config is assigned to one or more indexers".into(),
            ));
        }
        self.services
            .integrations
            .indexer_proxy_configs
            .delete(id)
            .await
    }

    pub async fn test_indexer_proxy_config(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<IndexerProxyTestResult> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let id = id.trim();
        if id.is_empty() {
            return Err(AppError::Validation("indexer proxy config id is required".into()));
        }
        let mut config = self
            .services
            .integrations
            .indexer_proxy_configs
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("indexer proxy config '{id}' not found")))?;

        let started = std::time::Instant::now();
        let result = probe_byparr_health(&config).await;
        let duration_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        let test_result = match result {
            Ok(message) => IndexerProxyTestResult {
                ok: true,
                status: scryer_domain::IndexerProxyHealthStatus::Healthy,
                message: Some(message),
                duration_ms: Some(duration_ms),
            },
            Err(error) => IndexerProxyTestResult {
                ok: false,
                status: scryer_domain::IndexerProxyHealthStatus::Unhealthy,
                message: Some(sanitize_indexer_proxy_error(&error.to_string())),
                duration_ms: Some(duration_ms),
            },
        };

        config.last_health_status = Some(test_result.status);
        config.last_error_message = if test_result.ok {
            None
        } else {
            test_result.message.clone()
        };
        config.last_error_at = (!test_result.ok).then(Utc::now);
        config.updated_at = Utc::now();
        let _ = self
            .services
            .integrations
            .indexer_proxy_configs
            .update(config)
            .await;

        Ok(test_result)
    }
}

fn normalize_indexer_proxy_name(raw: &str) -> AppResult<String> {
    let name = raw.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Validation("indexer proxy name is required".into()));
    }
    Ok(name)
}

fn normalize_indexer_proxy_base_url(raw: &str) -> AppResult<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AppError::Validation("indexer proxy base URL is required".into()));
    }
    let parsed = url::Url::parse(trimmed)
        .map_err(|error| AppError::Validation(format!("invalid indexer proxy base URL: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::Validation(
            "indexer proxy base URL must use http or https".into(),
        ));
    }
    if parsed.host_str().is_none_or(|host| host.trim().is_empty()) {
        return Err(AppError::Validation(
            "indexer proxy base URL must include a host".into(),
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_indexer_proxy_timeout(timeout: u32) -> AppResult<u32> {
    if !(1..=180).contains(&timeout) {
        return Err(AppError::Validation(
            "indexer proxy timeout must be between 1 and 180 seconds".into(),
        ));
    }
    Ok(timeout)
}

async fn probe_byparr_health(
    config: &scryer_domain::IndexerProxyConfig,
) -> AppResult<String> {
    if config.provider_type != scryer_domain::IndexerProxyProviderType::Byparr {
        return Err(AppError::Validation(
            "unsupported indexer proxy provider type".into(),
        ));
    }
    let url = format!("{}/health", config.base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(u64::from(
            config.request_timeout_seconds.saturating_add(5),
        )))
        .build()
        .map_err(|error| AppError::Repository(format!("failed to build Byparr client: {error}")))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| AppError::Repository(format!("Byparr health probe failed: {error}")))?;
    let status = response.status();
    if !status.is_success() {
        return Err(AppError::Repository(format!(
            "Byparr health probe returned HTTP {}",
            status.as_u16()
        )));
    }
    Ok(format!("Byparr health probe returned HTTP {}", status.as_u16()))
}

fn sanitize_indexer_proxy_error(message: &str) -> String {
    let mut sanitized = message.to_string();
    for marker in ["apikey=", "api_key=", "token=", "passkey=", "auth=", "rsskey=", "jwt="] {
        while let Some(start) = sanitized.to_ascii_lowercase().find(marker) {
            let value_start = start + marker.len();
            let value_end = sanitized[value_start..]
                .find(['&', ' ', '\'', '"'])
                .map(|offset| value_start + offset)
                .unwrap_or_else(|| sanitized.len());
            sanitized.replace_range(value_start..value_end, "REDACTED");
        }
    }
    sanitized
}
