const CATALOG_STATUS_KEY: &str = "plugin_catalog_redirect";
impl AppUseCase {
    pub async fn plugin_catalog_status(&self, actor: &User) -> AppResult<PluginCatalogStatus> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let now = Utc::now();
        let stored_status = self.load_stored_plugin_catalog_status_payload().await?;
        let primary_redirect_url = plugin_catalog_url();
        let github_redirect_url = fallback_plugin_catalog_url().to_string();
        let primary_available = fetch_plugin_bytes(
            &primary_redirect_url,
            "primary plugin catalog redirect",
            "plugin_catalog_status:primary",
        )
        .await
        .is_ok();
        let github_available = if primary_available {
            true
        } else {
            fetch_plugin_bytes(
                &github_redirect_url,
                "GitHub plugin catalog redirect",
                "plugin_catalog_status:github",
            )
            .await
            .is_ok()
        };
        let both_down = !primary_available && !github_available;
        let blocked_actions = if both_down {
            vec![
                "catalog_refresh".to_string(),
                "install".to_string(),
                "install_manual".to_string(),
                "upgrade".to_string(),
                "manual_repo_inspection".to_string(),
            ]
        } else {
            Vec::new()
        };
        let outage_message = both_down.then(|| {
            "Plugin catalog redirects are unavailable from both the primary CDN and the GitHub mirror."
                .to_string()
        });

        self.persist_plugin_catalog_status_payload(
            StoredPluginCatalogStatusPayload {
                github_available,
                blocked_actions: blocked_actions.clone(),
                message: outage_message.clone(),
                restore_warnings: stored_status.restore_warnings.clone(),
            },
            now,
        )
        .await?;

        let last_error = self
            .services
            .customization
            .plugin_installations
            .list_plugin_catalog_sources()
            .await?
            .into_iter()
            .find_map(|source| source.last_error);

        Ok(PluginCatalogStatus {
            refresh_state: if last_error.is_some() || both_down {
                "degraded".to_string()
            } else {
                "ready".to_string()
            },
            github_available,
            last_checked_at: Some(now.to_rfc3339()),
            outage_message,
            blocked_actions,
            restore_warnings: stored_status.restore_warnings,
            last_error,
        })
    }
}
impl AppUseCase {
    async fn load_stored_plugin_catalog_status_payload(
        &self,
    ) -> AppResult<StoredPluginCatalogStatusPayload> {
        let Some(record) = self
            .services
            .customization
            .plugin_installations
            .get_plugin_catalog_status(CATALOG_STATUS_KEY)
            .await?
        else {
            return Ok(StoredPluginCatalogStatusPayload::default());
        };

        serde_json::from_str(&record.status_json).map_err(|error| {
            AppError::Repository(format!(
                "failed to parse stored plugin catalog status '{}': {error}",
                record.status_key
            ))
        })
    }
}
impl AppUseCase {
    async fn persist_plugin_catalog_status_payload(
        &self,
        payload: StoredPluginCatalogStatusPayload,
        checked_at: chrono::DateTime<Utc>,
    ) -> AppResult<()> {
        let status_json = serde_json::to_string(&payload).map_err(|error| {
            AppError::Repository(format!(
                "failed to serialize plugin catalog status payload: {error}"
            ))
        })?;
        self.services
            .customization
            .plugin_installations
            .upsert_plugin_catalog_status(&PluginCatalogStatusRecord {
                status_key: CATALOG_STATUS_KEY.to_string(),
                status_json,
                checked_at,
            })
            .await
    }
}
