const MANAGED_INDEXER_SCOPE_IDS: &[&str] = &["movie", "series", "anime"];
fn normalize_managed_child_routing_scopes(
    scopes: Vec<ManagedIndexerRoutingScope>,
) -> AppResult<HashMap<String, Vec<String>>> {
    let mut routing_by_scope = HashMap::new();
    for scope in scopes {
        let scope_id = scope.scope_id.trim().to_ascii_lowercase();
        if !MANAGED_INDEXER_SCOPE_IDS.contains(&scope_id.as_str()) {
            return Err(AppError::Validation(format!(
                "managed child routing scope '{}' is not supported",
                scope.scope_id
            )));
        }
        if routing_by_scope.contains_key(&scope_id) {
            return Err(AppError::Validation(format!(
                "managed child routing contains duplicate scope '{}'",
                scope_id
            )));
        }
        routing_by_scope.insert(scope_id, normalize_routing_categories(scope.categories));
    }
    Ok(routing_by_scope)
}
fn apply_managed_child_routing(
    routing_by_scope: &mut HashMap<String, Vec<IndexerRoutingSettingsEntry>>,
    indexer_id: &str,
    desired_scopes: &HashMap<String, Vec<String>>,
) {
    for scope_id in MANAGED_INDEXER_SCOPE_IDS {
        let Some(categories) = desired_scopes.get(*scope_id).cloned() else {
            if let Some(entries) = routing_by_scope.get_mut(*scope_id) {
                entries.retain(|entry| entry.indexer_id != indexer_id);
            }
            continue;
        };
        upsert_indexer_routing_entry(
            routing_by_scope.entry((*scope_id).to_string()).or_default(),
            indexer_id,
            categories,
        );
    }
}
fn remove_indexer_routing_entries(
    routing_by_scope: &mut HashMap<String, Vec<IndexerRoutingSettingsEntry>>,
    indexer_id: &str,
) {
    for scope_id in MANAGED_INDEXER_SCOPE_IDS {
        if let Some(entries) = routing_by_scope.get_mut(*scope_id) {
            entries.retain(|entry| entry.indexer_id != indexer_id);
        }
    }
}
impl AppUseCase {
    pub fn queue_managed_indexer_sync(&self, config_id: &str) {
        let config_id = config_id.trim().to_string();
        if config_id.is_empty() {
            return;
        }

        let app = self.clone();
        tokio::spawn(async move {
            let actor = scryer_domain::User::new_admin("system-managed-indexer-sync");
            if let Err(error) = app.sync_indexer_config(&actor, &config_id).await {
                tracing::warn!(
                    config_id = %config_id,
                    error = %error,
                    "background managed indexer sync failed"
                );
            }
        });
    }
}
impl AppUseCase {
    async fn prepare_managed_indexer_sync_plan(
        &self,
        parent: &IndexerConfig,
        plan: IndexerSyncPlan,
    ) -> AppResult<Vec<PreparedManagedIndexerChild>> {
        let mut seen_child_keys = HashSet::new();
        let mut prepared = Vec::with_capacity(plan.children.len());

        for child in plan.children {
            let child_key = child.child_key.trim().to_string();
            if child_key.is_empty() {
                return Err(AppError::Validation(
                    "managed child plan entries require child_key".into(),
                ));
            }
            if !seen_child_keys.insert(child_key.clone()) {
                return Err(AppError::Validation(format!(
                    "managed child plan contains duplicate child_key '{}'",
                    child_key
                )));
            }

            let name = child.name.trim().to_string();
            if name.is_empty() {
                return Err(AppError::Validation(format!(
                    "managed child '{}' requires a name",
                    child_key
                )));
            }

            let provider_type = child.provider_type.trim().to_ascii_lowercase();
            if provider_type.is_empty() {
                return Err(AppError::Validation(format!(
                    "managed child '{}' requires provider_type",
                    child_key
                )));
            }

            let fields = self.indexer_config_fields_for_provider_type(&provider_type)?;
            let config_json =
                normalize_indexer_config_json(&fields, Some(child.config_json.as_str()), None)?;
            let base_url = derive_indexer_base_url_from_config_fields(&fields, Some(&config_json))?;
            let managed_metadata_json = child
                .managed_metadata_json
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let routing_by_scope = normalize_managed_child_routing_scopes(child.routing_scopes)?;

            prepared.push(PreparedManagedIndexerChild {
                child_key,
                name,
                provider_type,
                base_url,
                config_json,
                is_enabled: parent.is_enabled && child.is_enabled,
                enable_interactive_search: child.enable_interactive_search,
                enable_auto_search: child.enable_auto_search,
                managed_metadata_json,
                caps_snapshot_json: child
                    .caps_snapshot_json
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                routing_by_scope,
            });
        }

        Ok(prepared)
    }
}
