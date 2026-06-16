const CATALOG_STATUS_KEY: &str = "plugin_catalog_redirect";
impl AppUseCase {
    pub async fn plugin_catalog_status(&self, actor: &User) -> AppResult<PluginCatalogStatus> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let now = Utc::now();
        let stored_status = self.load_stored_plugin_catalog_status_payload().await?;
        let primary_redirect_url = plugin_catalog_url();
        let github_redirect_url = fallback_plugin_catalog_url().to_string();
        let primary_probe = fetch_verified_catalog_redirect_candidate(
            &primary_redirect_url,
            "primary plugin catalog redirect",
        )
        .await;
        let (primary_available, primary_error) = match primary_probe {
            Ok(_) => (true, None),
            Err(error) => (false, Some(error.to_string())),
        };
        let (github_available, github_error) = if primary_available {
            (true, None)
        } else {
            match fetch_verified_catalog_redirect_candidate(
                &github_redirect_url,
                "GitHub plugin catalog redirect",
            )
            .await
            {
                Ok(_) => (true, None),
                Err(error) => (false, Some(error.to_string())),
            }
        };
        let both_down = !primary_available && !github_available;
        let last_error = both_down
            .then(|| {
                combined_plugin_catalog_probe_error(
                    primary_error.as_deref(),
                    github_error.as_deref(),
                )
            })
            .flatten();
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

        Ok(PluginCatalogStatus {
            refresh_state: if both_down {
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

fn combined_plugin_catalog_probe_error(
    primary_error: Option<&str>,
    github_error: Option<&str>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(error) = primary_error.filter(|error| !error.trim().is_empty()) {
        parts.push(format!("primary plugin catalog redirect: {error}"));
    }
    if let Some(error) = github_error.filter(|error| !error.trim().is_empty()) {
        parts.push(format!("GitHub plugin catalog redirect: {error}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
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
