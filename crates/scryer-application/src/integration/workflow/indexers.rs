#[derive(Clone, Debug)]
struct PreparedManagedIndexerChild {
    child_key: String,
    name: String,
    provider_type: String,
    base_url: String,
    config_json: String,
    is_enabled: bool,
    enable_interactive_search: bool,
    enable_auto_search: bool,
    managed_metadata_json: Option<String>,
    caps_snapshot_json: Option<String>,
    routing_by_scope: HashMap<String, Vec<String>>,
}
fn merge_managed_caps_snapshot(existing: Option<&str>, desired: Option<&str>) -> Option<String> {
    let desired = desired?.trim();
    if desired.is_empty() {
        return None;
    }

    let mut desired_value = serde_json::from_str::<serde_json::Value>(desired).ok()?;
    let desired_object = desired_value.as_object_mut()?;
    if desired_object
        .get("caps_snapshot")
        .is_some_and(|value| !value.is_null())
    {
        return Some(desired.to_string());
    }

    let existing_snapshot = existing
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| value.as_object().cloned())
        .and_then(|object| object.get("caps_snapshot").cloned())
        .filter(|value| !value.is_null())?;

    desired_object.insert("caps_snapshot".to_string(), existing_snapshot);
    serde_json::to_string(&desired_value).ok()
}
fn next_indexer_routing_priority(entries: &[IndexerRoutingSettingsEntry]) -> i32 {
    entries
        .iter()
        .map(|entry| entry.priority)
        .max()
        .unwrap_or(0)
        + 1
}
fn upsert_indexer_routing_entry(
    entries: &mut Vec<IndexerRoutingSettingsEntry>,
    indexer_id: &str,
    categories: Vec<String>,
) {
    if let Some(entry) = entries
        .iter_mut()
        .find(|entry| entry.indexer_id == indexer_id)
    {
        entry.categories = categories;
        return;
    }

    entries.push(IndexerRoutingSettingsEntry {
        indexer_id: indexer_id.to_string(),
        enabled: true,
        categories,
        priority: next_indexer_routing_priority(entries),
    });
}
fn parse_indexer_config_json(
    config_json: Option<&str>,
) -> AppResult<serde_json::Map<String, serde_json::Value>> {
    let raw = config_json.unwrap_or_default().trim();
    if raw.is_empty() {
        return Ok(serde_json::Map::new());
    }

    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|error| AppError::Validation(error.to_string()))?;
    parsed
        .as_object()
        .cloned()
        .ok_or_else(|| AppError::Validation("indexer config_json must be a JSON object".into()))
}
fn indexer_connection_url_field(
    fields: &[scryer_domain::ConfigFieldDef],
) -> AppResult<&scryer_domain::ConfigFieldDef> {
    let mut connection_fields = fields
        .iter()
        .filter(|field| field.role == Some(scryer_domain::ConfigFieldRole::ConnectionUrl));
    let field = connection_fields.next().ok_or_else(|| {
        AppError::Validation("indexer provider is missing connection_url config field".into())
    })?;
    if connection_fields.next().is_some() {
        return Err(AppError::Validation(
            "indexer provider declares multiple connection_url config fields".into(),
        ));
    }
    Ok(field)
}
pub(crate) fn derive_indexer_base_url_from_config_fields(
    fields: &[scryer_domain::ConfigFieldDef],
    config_json: Option<&str>,
) -> AppResult<String> {
    let field = indexer_connection_url_field(fields)?;
    let object = parse_indexer_config_json(config_json)?;
    let raw = object
        .get(&field.key)
        .and_then(config_value_to_string)
        .or_else(|| {
            field
                .default_value
                .as_deref()
                .map(str::trim)
                .map(str::to_string)
        })
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Validation("indexer connection URL is required".into()))?;

    if (field.key.contains("feed") || field.key.contains("rss"))
        && let Some(origin) = extract_url_origin(&raw)
    {
        return Ok(origin);
    }

    Ok(raw)
}
pub(crate) fn normalize_indexer_config_json(
    fields: &[scryer_domain::ConfigFieldDef],
    config_json: Option<&str>,
    persisted_config_json: Option<&str>,
) -> AppResult<String> {
    indexer_connection_url_field(fields)?;

    let mut object = parse_indexer_config_json(config_json)?;
    let persisted = parse_indexer_config_json(persisted_config_json)?;

    for field in fields {
        let should_restore_persisted = match field.field_type {
            scryer_domain::ConfigFieldType::Password => {
                config_value_is_empty(object.get(&field.key))
            }
            _ => !object.contains_key(&field.key),
        };

        if should_restore_persisted
            && let Some(stored) = persisted.get(&field.key)
            && !config_value_is_empty(Some(stored))
        {
            object.insert(field.key.clone(), stored.clone());
        }

        if config_value_is_empty(object.get(&field.key))
            && let Some(default_value) = field
                .default_value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        {
            object.insert(
                field.key.clone(),
                serde_json::Value::String(default_value.to_string()),
            );
        }

        if field.required && config_value_is_empty(object.get(&field.key)) {
            return Err(AppError::Validation(format!(
                "{} is required",
                field.label.trim()
            )));
        }
    }

    serde_json::to_string(&serde_json::Value::Object(object))
        .map_err(|error| AppError::Repository(error.to_string()))
}
impl AppUseCase {
    pub fn indexer_config_fields_for_provider_type(
        &self,
        provider_type: &str,
    ) -> AppResult<Vec<scryer_domain::ConfigFieldDef>> {
        let normalized = provider_type.trim().to_lowercase();
        let Some(provider) = self.services.integrations.plugin_provider.available() else {
            return Err(AppError::Validation(
                "indexer provider is unavailable".into(),
            ));
        };
        if !provider
            .available_provider_types()
            .into_iter()
            .any(|value| value == normalized)
        {
            return Err(AppError::Validation(format!(
                "unsupported indexer provider type '{provider_type}'"
            )));
        }

        let fields = provider.config_fields_for_provider(&normalized);
        indexer_connection_url_field(&fields)?;
        Ok(fields)
    }
}
impl AppUseCase {
    fn indexer_management_capabilities_for_provider_type(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerManagementCapabilities {
        self.services
            .integrations
            .plugin_provider
            .available()
            .map(|provider| provider.management_capabilities_for_provider(provider_type))
            .unwrap_or_default()
    }
}
impl AppUseCase {
    async fn fetch_caps_snapshot_json_for_config(
        &self,
        config: &IndexerConfig,
    ) -> AppResult<Option<String>> {
        let Some(refresher) = self
            .services
            .integrations
            .indexer_caps_refresher
            .available()
        else {
            return Ok(None);
        };
        let Some(snapshot) = refresher.fetch_for_config(config).await? else {
            return Ok(None);
        };
        serde_json::to_string(&snapshot)
            .map(Some)
            .map_err(|error| AppError::Repository(error.to_string()))
    }
}
impl AppUseCase {
    pub(crate) async fn refresh_caps_snapshot_json_best_effort(
        &self,
        config: &IndexerConfig,
        fallback: Option<&str>,
    ) -> Option<String> {
        match self.fetch_caps_snapshot_json_for_config(config).await {
            Ok(Some(snapshot_json)) => Some(snapshot_json),
            Ok(None) => fallback.map(ToOwned::to_owned),
            Err(error) => {
                tracing::warn!(
                    config_id = %config.id,
                    provider_type = %config.provider_type,
                    error = %error,
                    "failed to refresh indexer caps snapshot; keeping the last known snapshot"
                );
                fallback.map(ToOwned::to_owned)
            }
        }
    }
}
impl AppUseCase {
    pub async fn list_indexer_configs(
        &self,
        actor: &User,
        provider_filter: Option<String>,
    ) -> AppResult<Vec<IndexerConfig>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .indexer_configs
            .list(provider_filter.map(|provider| provider.trim().to_lowercase()))
            .await
    }
}
impl AppUseCase {
    pub async fn refresh_enabled_direct_nab_caps_snapshots(
        &self,
        actor: &User,
    ) -> AppResult<(u32, Vec<String>)> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?;
        let mut refreshed = 0_u32;
        let mut failures = Vec::new();

        for config in configs {
            if !config.is_enabled || !config.is_direct_nab() {
                continue;
            }

            match self.fetch_caps_snapshot_json_for_config(&config).await {
                Ok(Some(snapshot_json)) => {
                    if config.caps_snapshot_json.as_deref() != Some(snapshot_json.as_str()) {
                        self.services
                            .integrations
                            .indexer_configs
                            .update(IndexerConfigUpdate {
                                id: config.id.clone(),
                                caps_snapshot_json: Some(Some(snapshot_json)),
                                ..Default::default()
                            })
                            .await?;
                    }
                    refreshed += 1;
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        config_id = %config.id,
                        provider_type = %config.provider_type,
                        error = %error,
                        "failed to refresh direct indexer caps snapshot"
                    );
                    failures.push(format!("{}: {}", config.name, error));
                }
            }
        }

        Ok((refreshed, failures))
    }
}
impl AppUseCase {
    pub async fn sync_enabled_prowlarr_indexers(
        &self,
        actor: &User,
    ) -> AppResult<(u32, Vec<String>)> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let parents = self
            .services
            .integrations
            .indexer_configs
            .list(Some("prowlarr".to_string()))
            .await?
            .into_iter()
            .filter(|config| config.managed_parent_config_id.is_none() && config.is_enabled)
            .collect::<Vec<_>>();

        let mut synced_count = 0;
        let mut failures = Vec::new();
        for parent in parents {
            match self.sync_indexer_config(actor, &parent.id).await {
                Ok(_) => synced_count += 1,
                Err(error) => failures.push(format!("{}: {error}", parent.name)),
            }
        }

        Ok((synced_count, failures))
    }
}
impl AppUseCase {
    pub async fn get_indexer_config(
        &self,
        actor: &User,
        config_id: &str,
    ) -> AppResult<Option<IndexerConfig>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .indexer_configs
            .get_by_id(config_id)
            .await
    }
}
impl AppUseCase {
    pub async fn create_indexer_config(
        &self,
        actor: &User,
        input: NewIndexerConfig,
    ) -> AppResult<IndexerConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::Validation("indexer name is required".into()));
        }

        let provider_type = input.provider_type.trim().to_lowercase();
        if provider_type.is_empty() {
            return Err(AppError::Validation("provider type is required".into()));
        }

        let fields = self.indexer_config_fields_for_provider_type(&provider_type)?;
        let management_capabilities =
            self.indexer_management_capabilities_for_provider_type(&provider_type);
        let normalized_config_json =
            normalize_indexer_config_json(&fields, input.config_json.as_deref(), None)?;
        let base_url =
            derive_indexer_base_url_from_config_fields(&fields, Some(&normalized_config_json))?;
        self.test_indexer_connection(actor, &provider_type, Some(&normalized_config_json), None)
            .await?;

        let mut config = IndexerConfig {
            id: Id::new().0,
            name,
            provider_type,
            base_url,
            api_key_encrypted: None,
            rate_limit_seconds: input.rate_limit_seconds,
            rate_limit_burst: input.rate_limit_burst,
            disabled_until: None,
            is_enabled: input.is_enabled,
            enable_interactive_search: if management_capabilities.supports_managed_children_sync {
                false
            } else {
                input.enable_interactive_search
            },
            enable_auto_search: if management_capabilities.supports_managed_children_sync {
                false
            } else {
                input.enable_auto_search
            },
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_at: None,
            config_json: Some(normalized_config_json),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        config.caps_snapshot_json = self
            .refresh_caps_snapshot_json_best_effort(&config, None)
            .await;

        let created = self
            .services
            .integrations
            .indexer_configs
            .create(config)
            .await?;
        self.ensure_indexer_routing_entry_for_indexer(actor, &created.id)
            .await?;
        if management_capabilities.supports_managed_children_sync && created.is_enabled {
            self.queue_managed_indexer_sync(&created.id);
        }
        self.publish_indexers_changed();
        Ok(created)
    }
}
impl AppUseCase {
    pub async fn update_indexer_config(
        &self,
        actor: &User,
        update: IndexerConfigUpdate,
    ) -> AppResult<IndexerConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let config_id = update.id.trim();
        if config_id.is_empty() {
            return Err(AppError::Validation("indexer config id is required".into()));
        }
        if !update.has_changes() {
            return Err(AppError::Validation(
                "at least one indexer field must be provided".into(),
            ));
        }

        let normalized_name = update.name.map(|value| value.trim().to_string());
        if normalized_name.as_ref().is_some_and(String::is_empty) {
            return Err(AppError::Validation("indexer name cannot be empty".into()));
        }

        let normalized_provider = update
            .provider_type
            .map(|value| value.trim().to_lowercase());
        if normalized_provider.as_ref().is_some_and(String::is_empty) {
            return Err(AppError::Validation("provider type cannot be empty".into()));
        }

        let existing = self
            .services
            .integrations
            .indexer_configs
            .get_by_id(config_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("indexer config '{config_id}' not found")))?;
        if existing.managed_parent_config_id.is_some() {
            return Err(AppError::Validation(
                "managed child indexers are controlled by their parent sync and cannot be edited directly"
                    .into(),
            ));
        }
        let effective_provider = normalized_provider
            .as_deref()
            .unwrap_or(existing.provider_type.as_str())
            .to_string();
        let fields = self.indexer_config_fields_for_provider_type(&effective_provider)?;
        let normalized_config_json = update
            .config_json
            .as_deref()
            .map(|raw| {
                normalize_indexer_config_json(&fields, Some(raw), existing.config_json.as_deref())
            })
            .transpose()?;
        let normalized_base_url =
            if normalized_config_json.is_some() || normalized_provider.is_some() {
                let config_source = normalized_config_json
                    .as_deref()
                    .or(existing.config_json.as_deref());
                Some(derive_indexer_base_url_from_config_fields(
                    &fields,
                    config_source,
                )?)
            } else {
                None
            };
        let management_capabilities =
            self.indexer_management_capabilities_for_provider_type(&effective_provider);
        let should_validate_connection = normalized_provider.is_some()
            || normalized_config_json.is_some()
            || matches!(update.is_enabled, Some(true)) && !existing.is_enabled;
        let should_sync_managed_children = management_capabilities.supports_managed_children_sync
            && updated_managed_parent_requires_sync(
                &existing,
                update.is_enabled,
                normalized_provider.is_some(),
                normalized_config_json.is_some(),
            );

        if should_validate_connection {
            let validation_config_json = normalized_config_json
                .as_deref()
                .or(existing.config_json.as_deref());
            self.test_indexer_connection(actor, &effective_provider, validation_config_json, None)
                .await?;
        }

        let preview_config = IndexerConfig {
            id: existing.id.clone(),
            name: normalized_name
                .clone()
                .unwrap_or_else(|| existing.name.clone()),
            provider_type: normalized_provider
                .clone()
                .unwrap_or_else(|| existing.provider_type.clone()),
            base_url: normalized_base_url
                .clone()
                .unwrap_or_else(|| existing.base_url.clone()),
            api_key_encrypted: existing.api_key_encrypted.clone(),
            rate_limit_seconds: update.rate_limit_seconds.or(existing.rate_limit_seconds),
            rate_limit_burst: update.rate_limit_burst.or(existing.rate_limit_burst),
            disabled_until: existing.disabled_until,
            is_enabled: update.is_enabled.unwrap_or(existing.is_enabled),
            enable_interactive_search: if management_capabilities.supports_managed_children_sync {
                false
            } else {
                update
                    .enable_interactive_search
                    .unwrap_or(existing.enable_interactive_search)
            },
            enable_auto_search: if management_capabilities.supports_managed_children_sync {
                false
            } else {
                update
                    .enable_auto_search
                    .unwrap_or(existing.enable_auto_search)
            },
            managed_parent_config_id: update
                .managed_parent_config_id
                .clone()
                .unwrap_or_else(|| existing.managed_parent_config_id.clone()),
            managed_child_key: update
                .managed_child_key
                .clone()
                .unwrap_or_else(|| existing.managed_child_key.clone()),
            managed_metadata_json: update
                .managed_metadata_json
                .clone()
                .unwrap_or_else(|| existing.managed_metadata_json.clone()),
            caps_snapshot_json: existing.caps_snapshot_json.clone(),
            last_health_status: existing.last_health_status.clone(),
            last_error_at: existing.last_error_at,
            config_json: normalized_config_json
                .clone()
                .or_else(|| existing.config_json.clone()),
            created_at: existing.created_at,
            updated_at: existing.updated_at,
        };
        let refreshed_caps_snapshot_json = self
            .refresh_caps_snapshot_json_best_effort(
                &preview_config,
                existing.caps_snapshot_json.as_deref(),
            )
            .await;

        let updated = self
            .services
            .integrations
            .indexer_configs
            .update(IndexerConfigUpdate {
                id: config_id.to_string(),
                name: normalized_name,
                provider_type: normalized_provider,
                derived_base_url: normalized_base_url,
                rate_limit_seconds: update.rate_limit_seconds,
                rate_limit_burst: update.rate_limit_burst,
                is_enabled: update.is_enabled,
                enable_interactive_search: if management_capabilities.supports_managed_children_sync
                {
                    Some(false)
                } else {
                    update.enable_interactive_search
                },
                enable_auto_search: if management_capabilities.supports_managed_children_sync {
                    Some(false)
                } else {
                    update.enable_auto_search
                },
                managed_parent_config_id: update.managed_parent_config_id,
                managed_child_key: update.managed_child_key,
                managed_metadata_json: update.managed_metadata_json,
                caps_snapshot_json: Some(refreshed_caps_snapshot_json),
                config_json: normalized_config_json,
            })
            .await?;
        if should_sync_managed_children {
            if updated.is_enabled {
                self.queue_managed_indexer_sync(&updated.id);
            } else if existing.is_enabled != updated.is_enabled
                && let Err(error) = self
                    .set_managed_child_indexers_enabled_state(&updated.id, false)
                    .await
            {
                self.publish_indexers_changed();
                return Err(error);
            }
        }
        self.publish_indexers_changed();
        Ok(updated)
    }
}
impl AppUseCase {
    pub async fn delete_indexer_config(&self, actor: &User, config_id: &str) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let config_id = config_id.trim();
        let config = self
            .services
            .integrations
            .indexer_configs
            .get_by_id(config_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("indexer config '{config_id}' not found")))?;
        if config.managed_parent_config_id.is_some() {
            return Err(AppError::Validation(
                "managed child indexers are controlled by their parent sync".into(),
            ));
        }

        let children = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|candidate| {
                candidate.managed_parent_config_id.as_deref() == Some(config.id.as_str())
            })
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>();
        let mut routing_by_scope = self.load_indexer_routing_by_scope(actor).await?;
        for child_id in &children {
            self.services
                .integrations
                .indexer_configs
                .delete(child_id)
                .await?;
            remove_indexer_routing_entries(&mut routing_by_scope, child_id);
        }
        self.services
            .integrations
            .indexer_configs
            .delete(&config.id)
            .await?;
        remove_indexer_routing_entries(&mut routing_by_scope, &config.id);
        self.save_indexer_routing_by_scope(actor, routing_by_scope)
            .await?;
        self.publish_indexers_changed();
        Ok(())
    }
}
impl AppUseCase {
    pub async fn sync_indexer_config(
        &self,
        actor: &User,
        config_id: &str,
    ) -> AppResult<IndexerConfigSyncResult> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let config_id = config_id.trim();
        if config_id.is_empty() {
            return Err(AppError::Validation("indexer config id is required".into()));
        }

        let _sync_guard = self
            .runtime
            .integrations
            .managed_indexer_sync_lock
            .clone()
            .lock_owned()
            .await;
        let mut indexers_changed = false;
        macro_rules! try_sync_step {
            ($expr:expr) => {
                match $expr {
                    Ok(value) => value,
                    Err(error) => {
                        if indexers_changed {
                            self.publish_indexers_changed();
                        }
                        return Err(error);
                    }
                }
            };
        }

        let parent = self
            .services
            .integrations
            .indexer_configs
            .get_by_id(config_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("indexer config '{config_id}' not found")))?;
        if parent.managed_parent_config_id.is_some() {
            return Err(AppError::Validation(
                "managed child indexers cannot be synced directly".into(),
            ));
        }

        let provider = self
            .services
            .integrations
            .plugin_provider
            .available()
            .ok_or_else(|| AppError::Repository("indexer provider not available".into()))?;
        let management_capabilities =
            provider.management_capabilities_for_provider(&parent.provider_type);
        if !management_capabilities.supports_managed_children_sync {
            return Err(AppError::Validation(format!(
                "provider type '{}' does not support managed child sync",
                parent.provider_type
            )));
        }

        let parent = if parent.enable_interactive_search || parent.enable_auto_search {
            let updated = self
                .services
                .integrations
                .indexer_configs
                .update(IndexerConfigUpdate {
                    id: parent.id.clone(),
                    enable_interactive_search: Some(false),
                    enable_auto_search: Some(false),
                    ..Default::default()
                })
                .await?;
            indexers_changed = true;
            updated
        } else {
            parent
        };

        let client = try_sync_step!(provider.management_client_for_provider(&parent).ok_or_else(
            || {
                AppError::Validation(format!(
                    "no indexer management client available for provider type '{}'",
                    parent.provider_type
                ))
            }
        ));

        let plan = try_sync_step!(client.plan_sync(&parent.id).await);
        let desired_children =
            try_sync_step!(self.prepare_managed_indexer_sync_plan(&parent, plan).await);
        let existing_children =
            try_sync_step!(self.services.integrations.indexer_configs.list(None).await)
                .into_iter()
                .filter(|candidate| {
                    candidate.managed_parent_config_id.as_deref() == Some(parent.id.as_str())
                })
                .collect::<Vec<_>>();
        let mut existing_by_key = existing_children
            .into_iter()
            .filter_map(|candidate| {
                candidate
                    .managed_child_key
                    .clone()
                    .map(|child_key| (child_key, candidate))
            })
            .collect::<HashMap<_, _>>();
        let mut routing_by_scope = try_sync_step!(self.load_indexer_routing_by_scope(actor).await);
        let mut result = IndexerConfigSyncResult {
            parent_config_id: parent.id.clone(),
            ..Default::default()
        };

        for desired in desired_children {
            if let Some(existing) = existing_by_key.remove(&desired.child_key) {
                let managed_metadata_json = merge_managed_caps_snapshot(
                    existing.managed_metadata_json.as_deref(),
                    desired.managed_metadata_json.as_deref(),
                )
                .or_else(|| desired.managed_metadata_json.clone());
                let updated = try_sync_step!(
                    self.services
                        .integrations
                        .indexer_configs
                        .update(IndexerConfigUpdate {
                            id: existing.id.clone(),
                            name: Some(desired.name.clone()),
                            provider_type: Some(desired.provider_type.clone()),
                            derived_base_url: Some(desired.base_url.clone()),
                            rate_limit_seconds: None,
                            rate_limit_burst: None,
                            is_enabled: Some(desired.is_enabled),
                            enable_interactive_search: Some(desired.enable_interactive_search),
                            enable_auto_search: Some(desired.enable_auto_search),
                            managed_parent_config_id: Some(Some(parent.id.clone())),
                            managed_child_key: Some(Some(desired.child_key.clone())),
                            managed_metadata_json: Some(managed_metadata_json),
                            caps_snapshot_json: Some(desired.caps_snapshot_json.clone()),
                            config_json: Some(desired.config_json.clone()),
                        })
                        .await
                );
                indexers_changed = true;
                apply_managed_child_routing(
                    &mut routing_by_scope,
                    &updated.id,
                    &desired.routing_by_scope,
                );
                result.updated_ids.push(updated.id);
            } else {
                let created = try_sync_step!(
                    self.services
                        .integrations
                        .indexer_configs
                        .create(IndexerConfig {
                            id: Id::new().0,
                            name: desired.name.clone(),
                            provider_type: desired.provider_type.clone(),
                            base_url: desired.base_url.clone(),
                            api_key_encrypted: None,
                            rate_limit_seconds: None,
                            rate_limit_burst: None,
                            disabled_until: None,
                            is_enabled: desired.is_enabled,
                            enable_interactive_search: desired.enable_interactive_search,
                            enable_auto_search: desired.enable_auto_search,
                            managed_parent_config_id: Some(parent.id.clone()),
                            managed_child_key: Some(desired.child_key.clone()),
                            managed_metadata_json: desired.managed_metadata_json.clone(),
                            caps_snapshot_json: desired.caps_snapshot_json.clone(),
                            last_health_status: None,
                            last_error_at: None,
                            config_json: Some(desired.config_json.clone()),
                            created_at: Utc::now(),
                            updated_at: Utc::now(),
                        })
                        .await
                );
                indexers_changed = true;
                apply_managed_child_routing(
                    &mut routing_by_scope,
                    &created.id,
                    &desired.routing_by_scope,
                );
                result.created_ids.push(created.id);
            }
        }

        for (_, obsolete) in existing_by_key {
            try_sync_step!(
                self.services
                    .integrations
                    .indexer_configs
                    .delete(&obsolete.id)
                    .await
            );
            indexers_changed = true;
            remove_indexer_routing_entries(&mut routing_by_scope, &obsolete.id);
            result.deleted_ids.push(obsolete.id);
        }

        try_sync_step!(
            self.save_indexer_routing_by_scope(actor, routing_by_scope)
                .await
        );
        if indexers_changed {
            self.publish_indexers_changed();
        }
        Ok(result)
    }
}
impl AppUseCase {
    async fn set_managed_child_indexers_enabled_state(
        &self,
        parent_config_id: &str,
        is_enabled: bool,
    ) -> AppResult<()> {
        let children = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|candidate| {
                candidate.managed_parent_config_id.as_deref() == Some(parent_config_id)
                    && candidate.is_enabled != is_enabled
            })
            .collect::<Vec<_>>();

        for child in children {
            self.services
                .integrations
                .indexer_configs
                .update(IndexerConfigUpdate {
                    id: child.id,
                    is_enabled: Some(is_enabled),
                    ..Default::default()
                })
                .await?;
        }

        Ok(())
    }
}
impl AppUseCase {
    async fn load_indexer_routing_by_scope(
        &self,
        actor: &User,
    ) -> AppResult<HashMap<String, Vec<IndexerRoutingSettingsEntry>>> {
        let mut routing_by_scope = HashMap::new();
        for scope_id in MANAGED_INDEXER_SCOPE_IDS {
            routing_by_scope.insert(
                scope_id.to_string(),
                self.get_indexer_routing(actor, scope_id).await?,
            );
        }
        Ok(routing_by_scope)
    }
}
impl AppUseCase {
    async fn save_indexer_routing_by_scope(
        &self,
        actor: &User,
        mut routing_by_scope: HashMap<String, Vec<IndexerRoutingSettingsEntry>>,
    ) -> AppResult<()> {
        for scope_id in MANAGED_INDEXER_SCOPE_IDS {
            let entries = routing_by_scope.remove(*scope_id).unwrap_or_default();
            self.update_indexer_routing(actor, scope_id, entries)
                .await?;
        }
        Ok(())
    }
}
fn updated_managed_parent_requires_sync(
    existing: &IndexerConfig,
    updated_enabled_state: Option<bool>,
    provider_changed: bool,
    config_changed: bool,
) -> bool {
    if !existing.is_enabled && !matches!(updated_enabled_state, Some(true)) {
        return false;
    }

    provider_changed
        || config_changed
        || (matches!(updated_enabled_state, Some(true)) && !existing.is_enabled)
        || (matches!(updated_enabled_state, Some(false)) && existing.is_enabled)
}
fn merge_tracked_download_background_work_state(
    tracked: &mut crate::tracked_downloads::TrackedDownload,
    finished: crate::tracked_downloads::TrackedDownload,
) {
    tracked.state = finished.state;
    tracked.status = finished.status;
    tracked.status_messages = finished.status_messages;
    tracked.title_id = finished.title_id;
    tracked.facet = finished.facet;
    tracked.source_title = finished.source_title;
    tracked.indexer = finished.indexer;
    tracked.added_at = finished.added_at;
    tracked.notified_manual_interaction = finished.notified_manual_interaction;
    tracked.match_type = finished.match_type;
    tracked.import_attempted = finished.import_attempted;
    tracked.path_missing_since = finished.path_missing_since;
}
