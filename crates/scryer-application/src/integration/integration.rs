use super::*;
use crate::domain_events::new_download_queue_domain_event;
use crate::event_views::{apply_download_queue_projection_event, sorted_download_queue_items};
use crate::tracked_downloads::{
    TrackedDownload, TrackedDownloadQueueMetadata, tracked_download_id,
};
use scryer_domain::{
    DomainEventFilter, DomainEventPayload, DomainEventType, DownloadQueueItemRemovedEventData,
    DownloadQueueItemUpsertedEventData, ImportType,
};
use std::collections::{HashMap, HashSet};

fn extract_url_origin(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let (scheme, remainder) = trimmed.split_once("://")?;
    if scheme.is_empty() {
        return None;
    }

    let authority = remainder.split(['/', '?', '#']).next()?.trim();
    if authority.is_empty() {
        return None;
    }

    Some(format!("{scheme}://{authority}"))
}

fn apply_import_record_to_queue_item(item: &mut DownloadQueueItem, record: &ImportRecord) {
    item.import_status = Some(record.status);
    item.imported_at = record
        .finished_at
        .clone()
        .or(Some(record.updated_at.clone()));

    if let Some(result_json) = record.result_json.as_deref()
        && let Ok(result) = serde_json::from_str::<scryer_domain::ImportResult>(result_json)
        && let Some(error_msg) = result.error_message
    {
        item.import_error_message = Some(error_msg.clone());
        item.attention_reason = Some(error_msg);
    }
}

fn apply_manual_import_record_to_queue_item(item: &mut DownloadQueueItem, record: &ImportRecord) {
    item.import_status = Some(record.status);
    item.imported_at = record
        .finished_at
        .clone()
        .or(Some(record.updated_at.clone()));

    if let Some(result_json) = record.result_json.as_deref()
        && let Ok(result) = serde_json::from_str::<crate::ManualImportExecutionResult>(result_json)
    {
        item.import_error_code = result.error_code;
        item.import_error_message = result.error_message.clone();
        if let Some(message) = result.error_message {
            item.attention_reason = Some(message);
        }
    }
}

fn queue_item_import_state_eligible(item: &DownloadQueueItem) -> bool {
    matches!(
        item.state,
        DownloadQueueState::Completed
            | DownloadQueueState::Failed
            | DownloadQueueState::ImportPending
    )
}

async fn enrich_queue_item_import_states(app: &AppUseCase, items: &mut [DownloadQueueItem]) {
    let sources = items
        .iter()
        .filter(|item| queue_item_import_state_eligible(item))
        .map(|item| {
            (
                item.client_type.clone(),
                item.download_client_item_id.clone(),
            )
        })
        .collect::<Vec<_>>();

    if sources.is_empty() {
        return;
    }

    let records = match app
        .services
        .workflow
        .imports
        .list_imports_for_sources(&sources)
        .await
    {
        Ok(records) => records,
        Err(error) => {
            tracing::warn!(error = %error, "failed to batch-load import state for queue items");
            return;
        }
    };

    let mut manual_records = HashMap::new();
    let mut fallback_records = HashMap::new();
    for record in records {
        let key = (record.source_system.clone(), record.source_ref.clone());
        if record.import_type == ImportType::ManualImport {
            manual_records.entry(key).or_insert(record);
        } else {
            fallback_records.entry(key).or_insert(record);
        }
    }

    for item in items
        .iter_mut()
        .filter(|item| queue_item_import_state_eligible(item))
    {
        let key = (
            item.client_type.clone(),
            item.download_client_item_id.clone(),
        );
        if let Some(record) = manual_records.get(&key) {
            apply_manual_import_record_to_queue_item(item, record);
            continue;
        }
        if let Some(record) = fallback_records.get(&key) {
            apply_import_record_to_queue_item(item, record);
        }
    }
}

fn derive_indexer_base_url_from_config_json(config_json: Option<&str>) -> Option<String> {
    let raw = config_json?.trim();
    if raw.is_empty() {
        return None;
    }

    let parsed: serde_json::Value = serde_json::from_str(raw).ok()?;
    let object = parsed.as_object()?;

    for key in ["feed_url", "feedUrl", "rss_url", "rssUrl"] {
        if let Some(value) = object.get(key).and_then(|value| value.as_str())
            && let Some(origin) = extract_url_origin(value)
        {
            return Some(origin);
        }
    }

    None
}

pub(crate) fn resolve_indexer_base_url(
    base_url: &str,
    config_json: Option<&str>,
) -> AppResult<String> {
    let normalized = base_url.trim();
    if !normalized.is_empty() {
        return Ok(normalized.to_string());
    }

    derive_indexer_base_url_from_config_json(config_json)
        .ok_or_else(|| AppError::Validation("base URL is required".into()))
}

fn download_queue_projection_key(item: &DownloadQueueItem) -> String {
    format!("{}::{}", item.client_type, item.download_client_item_id)
}

fn apply_tracked_download_queue_metadata(
    item: &mut DownloadQueueItem,
    tracked: &TrackedDownloadQueueMetadata,
) {
    item.tracked_state = Some(tracked.state);
    item.tracked_status = Some(tracked.status);
    item.tracked_status_messages
        .clone_from(&tracked.status_messages);
    item.tracked_match_type = Some(tracked.match_type);
    if item.title_id.is_none() && tracked.title_id.is_some() {
        item.title_id.clone_from(&tracked.title_id);
    }
    if item.facet.is_none() && tracked.facet.is_some() {
        item.facet.clone_from(&tracked.facet);
    }
}

fn tracked_download_queue_snapshot(item: &TrackedDownload) -> TrackedDownloadQueueMetadata {
    TrackedDownloadQueueMetadata::from(item)
}

pub async fn publish_download_queue_snapshot_events(
    app: &AppUseCase,
    actor_user_id: Option<String>,
    previous_items: &mut HashMap<String, DownloadQueueItem>,
    items: &[DownloadQueueItem],
) {
    let mut next_items = HashMap::with_capacity(items.len());
    let mut domain_events = Vec::new();

    for item in items {
        let key = download_queue_projection_key(item);
        let changed = previous_items
            .get(&key)
            .is_none_or(|previous| previous != item);
        if changed {
            domain_events.push(new_download_queue_domain_event(
                actor_user_id.clone(),
                key.clone(),
                DomainEventPayload::DownloadQueueItemUpserted(DownloadQueueItemUpsertedEventData {
                    item: item.clone(),
                }),
            ));
        }
        next_items.insert(key, item.clone());
    }

    for (key, previous_item) in previous_items.iter() {
        if !next_items.contains_key(key) {
            domain_events.push(new_download_queue_domain_event(
                actor_user_id.clone(),
                key.clone(),
                DomainEventPayload::DownloadQueueItemRemoved(DownloadQueueItemRemovedEventData {
                    download_client_item_id: previous_item.download_client_item_id.clone(),
                    client_type: Some(previous_item.client_type.clone()),
                }),
            ));
        }
    }

    *previous_items = next_items;

    if !domain_events.is_empty()
        && let Err(error) = app.append_domain_events(domain_events).await
    {
        tracing::warn!(error = %error, "failed to append download queue domain events");
    }
}

impl AppUseCase {
    fn normalize_download_client_type(&self, client_type: impl AsRef<str>) -> AppResult<String> {
        let normalized = client_type.as_ref().trim().to_lowercase();
        if normalized.is_empty() {
            return Err(AppError::Validation("client type is required".into()));
        }

        if NATIVE_DOWNLOAD_CLIENT_TYPES
            .iter()
            .any(|value| value.eq(&normalized.as_str()))
        {
            return Ok(normalized);
        }

        if self
            .services
            .integrations
            .download_client_plugin_provider
            .available()
            .is_some_and(|provider| {
                provider
                    .available_provider_types()
                    .into_iter()
                    .any(|value| value == normalized)
            })
        {
            return Ok(normalized);
        }

        Err(AppError::Validation(format!(
            "unsupported download client type '{}'",
            client_type.as_ref()
        )))
    }

    fn normalize_download_client_config_json(&self, raw: impl AsRef<str>) -> AppResult<String> {
        let raw = raw.as_ref().trim();
        if raw.is_empty() {
            return Ok("{}".to_string());
        }

        let parsed: serde_json::Value =
            serde_json::from_str(raw).map_err(|error| AppError::Validation(error.to_string()))?;
        serde_json::to_string(&parsed).map_err(|error| AppError::Repository(error.to_string()))
    }

    pub async fn list_indexer_configs(
        &self,
        actor: &User,
        provider_filter: Option<String>,
    ) -> AppResult<Vec<IndexerConfig>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.services
            .integrations
            .indexer_configs
            .list(provider_filter.map(|provider| provider.trim().to_lowercase()))
            .await
    }

    pub async fn get_indexer_config(
        &self,
        actor: &User,
        config_id: &str,
    ) -> AppResult<Option<IndexerConfig>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.services
            .integrations
            .indexer_configs
            .get_by_id(config_id)
            .await
    }

    pub async fn create_indexer_config(
        &self,
        actor: &User,
        input: NewIndexerConfig,
    ) -> AppResult<IndexerConfig> {
        require(actor, &Entitlement::ManageConfig)?;

        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::Validation("indexer name is required".into()));
        }

        let provider_type = input.provider_type.trim().to_lowercase();
        if provider_type.is_empty() {
            return Err(AppError::Validation("provider type is required".into()));
        }

        let normalized_config_json = input
            .config_json
            .clone()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let base_url =
            resolve_indexer_base_url(&input.base_url, normalized_config_json.as_deref())?;

        let api_key_encrypted = input
            .api_key_encrypted
            .map(|value| value.trim().to_string())
            .and_then(|value| if value.is_empty() { None } else { Some(value) });

        if let Some(value) = api_key_encrypted.as_deref()
            && value.len() < 8
        {
            return Err(AppError::Validation(
                "api key appears too short; provide a valid key".into(),
            ));
        }

        let config = IndexerConfig {
            id: Id::new().0,
            name,
            provider_type,
            base_url,
            api_key_encrypted,
            rate_limit_seconds: input.rate_limit_seconds,
            rate_limit_burst: input.rate_limit_burst,
            disabled_until: None,
            is_enabled: input.is_enabled,
            enable_interactive_search: input.enable_interactive_search,
            enable_auto_search: input.enable_auto_search,
            last_health_status: None,
            last_error_at: None,
            config_json: normalized_config_json,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.services
            .integrations
            .indexer_configs
            .create(config)
            .await
    }

    pub async fn update_indexer_config(
        &self,
        actor: &User,
        update: IndexerConfigUpdate,
    ) -> AppResult<IndexerConfig> {
        require(actor, &Entitlement::ManageConfig)?;
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

        let normalized_config_json = update.config_json.map(|value| value.trim().to_string());

        let normalized_base_url = match update.base_url {
            Some(value) => {
                let normalized = value.trim().to_string();
                if normalized.is_empty() {
                    return Err(AppError::Validation("base URL cannot be empty".into()));
                }
                Some(normalized)
            }
            None => derive_indexer_base_url_from_config_json(normalized_config_json.as_deref()),
        };

        let normalized_api_key = update
            .api_key_encrypted
            .map(|value| value.trim().to_string())
            .and_then(|value| if value.is_empty() { None } else { Some(value) });

        if let Some(value) = normalized_api_key.as_ref()
            && value.len() < 8
        {
            return Err(AppError::Validation(
                "api key appears too short; provide a valid key".into(),
            ));
        }

        let updated = self
            .services
            .integrations
            .indexer_configs
            .update(IndexerConfigUpdate {
                id: config_id.to_string(),
                name: normalized_name,
                provider_type: normalized_provider,
                base_url: normalized_base_url,
                api_key_encrypted: normalized_api_key,
                rate_limit_seconds: update.rate_limit_seconds,
                rate_limit_burst: update.rate_limit_burst,
                is_enabled: update.is_enabled,
                enable_interactive_search: update.enable_interactive_search,
                enable_auto_search: update.enable_auto_search,
                config_json: normalized_config_json,
            })
            .await?;
        Ok(updated)
    }

    pub async fn delete_indexer_config(&self, actor: &User, config_id: &str) -> AppResult<()> {
        require(actor, &Entitlement::ManageConfig)?;
        self.services
            .integrations
            .indexer_configs
            .delete(config_id)
            .await?;
        Ok(())
    }

    pub async fn list_download_client_configs(
        &self,
        actor: &User,
        client_type: Option<String>,
    ) -> AppResult<Vec<DownloadClientConfig>> {
        require(actor, &Entitlement::ManageConfig)?;

        let client_type = client_type
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(value) = client_type.as_deref() {
            self.normalize_download_client_type(value)?;
        }

        self.services
            .integrations
            .download_client_configs
            .list(client_type)
            .await
    }

    async fn collect_download_queue_items(
        &self,
        include_all_activity: bool,
        include_history_only: bool,
        use_tracked_runtime_snapshot: bool,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let mut enabled_clients = self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|item| item.is_enabled)
            .collect::<Vec<_>>();

        if enabled_clients.is_empty() {
            return Ok(vec![]);
        }

        enabled_clients.sort_by_key(|config| config.client_priority);
        let primary_client = enabled_clients
            .into_iter()
            .next()
            .ok_or_else(|| AppError::NotFound("no enabled download clients".to_string()))?;

        let queue_items = if include_history_only {
            Vec::new()
        } else {
            self.services
                .integrations
                .download_client
                .list_queue()
                .await?
        };
        let history_items = if include_history_only || include_all_activity {
            self.services
                .integrations
                .download_client
                .list_history()
                .await?
        } else {
            Vec::new()
        };

        let mut items: Vec<DownloadQueueItem> = queue_items;
        items.extend(history_items);

        // Enrich items with download_submissions data (for SABnzbd which
        // cannot embed metadata in the download itself). This populates
        // title_id, facet, and is_scryer_origin from the submissions table.
        for item in &mut items {
            if item.is_scryer_origin {
                continue;
            }
            if let Ok(Some(submission)) = self
                .services
                .workflow
                .download_submissions
                .find_by_client_item_id(&item.client_type, &item.download_client_item_id)
                .await
                && !submission.title_id.trim().is_empty()
            {
                item.is_scryer_origin = true;
                item.title_id = Some(submission.title_id);
                item.facet = Some(submission.facet);
            }
        }

        if use_tracked_runtime_snapshot
            && let Some(handle) = self.runtime.tracked_download_handle.as_ref()
        {
            let tracked_ids = items
                .iter()
                .map(|item| tracked_download_id(&item.client_type, &item.download_client_item_id))
                .collect::<Vec<_>>();

            match handle.snapshot(tracked_ids).await {
                Ok(snapshot) => {
                    for item in &mut items {
                        let tracked_id =
                            tracked_download_id(&item.client_type, &item.download_client_item_id);
                        if let Some(metadata) = snapshot.get(&tracked_id) {
                            apply_tracked_download_queue_metadata(item, metadata);
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(error = %error, "failed to load tracked download queue snapshot");
                }
            }
        }

        let items = dedupe_download_queue_items(items);

        let merged = items
            .into_iter()
            .filter(|item| include_history_only || include_all_activity || item.is_scryer_origin)
            .filter(|item| {
                if include_all_activity {
                    true
                } else if include_history_only {
                    is_history_download_state(&item.state)
                } else {
                    is_active_download_state(&item.state)
                }
            })
            .map(|item| {
                let mut mapped = item;
                if mapped.client_id.is_empty() {
                    mapped.client_id = primary_client.id.clone();
                }
                if mapped.client_name.is_empty() {
                    mapped.client_name = primary_client.name.clone();
                }
                if mapped.client_type.is_empty() {
                    mapped.client_type = primary_client.client_type.clone();
                }
                mapped.attention_required = matches!(
                    mapped.state,
                    DownloadQueueState::Failed | DownloadQueueState::ImportPending
                );
                if mapped.attention_reason.is_none() {
                    mapped.attention_reason = if mapped.attention_required {
                        Some("requires attention".to_string())
                    } else {
                        None
                    };
                }
                mapped
            })
            .collect::<Vec<_>>();

        let mut merged = merged;

        if include_history_only {
            merged.sort_by(|left, right| {
                parse_sort_value(
                    right.last_updated_at.as_deref(),
                    left.last_updated_at.as_deref(),
                )
            });
            merged.truncate(50);
        } else {
            // Enrich completed/failed items with import status from the imports table
            merged.sort_by(|left, right| {
                let left_rank = queue_state_sort_rank(&left.state);
                let right_rank = queue_state_sort_rank(&right.state);
                if left_rank != right_rank {
                    return left_rank.cmp(&right_rank);
                }

                match left.state {
                    DownloadQueueState::Downloading => right
                        .progress_percent
                        .cmp(&left.progress_percent)
                        .then_with(|| left.id.cmp(&right.id)),
                    DownloadQueueState::Queued | DownloadQueueState::Paused => {
                        parse_sort_value(left.queued_at.as_deref(), right.queued_at.as_deref())
                    }
                    _ => parse_sort_value(
                        left.last_updated_at.as_deref(),
                        right.last_updated_at.as_deref(),
                    )
                    .reverse(),
                }
            });
        }

        // Enrich completed/failed items with import status from the imports table
        enrich_queue_item_import_states(self, &mut merged).await;

        Ok(merged)
    }

    pub async fn list_download_queue(
        &self,
        actor: &User,
        include_all_activity: bool,
        include_history_only: bool,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.collect_download_queue_items(include_all_activity, include_history_only, true)
            .await
    }

    pub async fn list_download_history_page(
        &self,
        actor: &User,
        limit: usize,
        offset: usize,
    ) -> AppResult<DownloadHistoryPage> {
        require(actor, &Entitlement::ManageConfig)?;

        let limit = limit.clamp(1, 100);
        let fetch_limit = limit.saturating_add(1);

        let mut enabled_clients = self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|item| item.is_enabled)
            .collect::<Vec<_>>();

        let primary_client = if enabled_clients.is_empty() {
            return Ok(DownloadHistoryPage {
                items: Vec::new(),
                has_more: false,
            });
        } else {
            enabled_clients.sort_by_key(|config| config.client_priority);
            enabled_clients.into_iter().next()
        };

        let mut items = self
            .services
            .integrations
            .download_client
            .list_history_page(offset, fetch_limit)
            .await?;

        for item in &mut items {
            if item.is_scryer_origin {
                continue;
            }
            if let Ok(Some(submission)) = self
                .services
                .workflow
                .download_submissions
                .find_by_client_item_id(&item.client_type, &item.download_client_item_id)
                .await
                && !submission.title_id.trim().is_empty()
            {
                item.is_scryer_origin = true;
                item.title_id = Some(submission.title_id);
                item.facet = Some(submission.facet);
            }
        }

        if let Some(handle) = self.runtime.tracked_download_handle.as_ref() {
            let tracked_ids = items
                .iter()
                .map(|item| tracked_download_id(&item.client_type, &item.download_client_item_id))
                .collect::<Vec<_>>();

            match handle.snapshot(tracked_ids).await {
                Ok(snapshot) => {
                    for item in &mut items {
                        let tracked_id =
                            tracked_download_id(&item.client_type, &item.download_client_item_id);
                        if let Some(metadata) = snapshot.get(&tracked_id) {
                            apply_tracked_download_queue_metadata(item, metadata);
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(error = %error, "failed to load tracked download history snapshot");
                }
            }
        }

        let mut items = dedupe_download_queue_items(items)
            .into_iter()
            .filter(|item| is_history_download_state(&item.state))
            .map(|item| {
                let mut mapped = item;
                if mapped.client_id.is_empty()
                    && let Some(primary_client) = primary_client.as_ref()
                {
                    mapped.client_id = primary_client.id.clone();
                }
                if mapped.client_name.is_empty()
                    && let Some(primary_client) = primary_client.as_ref()
                {
                    mapped.client_name = primary_client.name.clone();
                }
                if mapped.client_type.is_empty()
                    && let Some(primary_client) = primary_client.as_ref()
                {
                    mapped.client_type = primary_client.client_type.clone();
                }
                mapped.attention_required = matches!(
                    mapped.state,
                    DownloadQueueState::Failed | DownloadQueueState::ImportPending
                );
                if mapped.attention_reason.is_none() {
                    mapped.attention_reason = if mapped.attention_required {
                        Some("requires attention".to_string())
                    } else {
                        None
                    };
                }
                mapped
            })
            .collect::<Vec<_>>();

        enrich_queue_item_import_states(self, &mut items).await;

        let has_more = items.len() > limit;
        items.truncate(limit);

        Ok(DownloadHistoryPage { items, has_more })
    }

    pub async fn find_download_queue_item(
        &self,
        actor: &User,
        client_type: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<Option<DownloadQueueItem>> {
        require(actor, &Entitlement::TriggerActions)?;

        let target_download_client_item_id = download_client_item_id.trim();
        if target_download_client_item_id.is_empty() {
            return Err(AppError::Validation(
                "download client item id is required".to_string(),
            ));
        }

        let normalized_client_type = client_type
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());

        let items = self.collect_download_queue_items(true, false, true).await?;
        Ok(items.into_iter().find(|item| {
            item.download_client_item_id == target_download_client_item_id
                && normalized_client_type
                    .as_ref()
                    .is_none_or(|client_type| item.client_type.eq_ignore_ascii_case(client_type))
        }))
    }

    pub fn subscribe_download_queue(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<Vec<DownloadQueueItem>>> {
        require(actor, &Entitlement::ManageConfig)?;
        let (tx, rx) = broadcast::channel(32);
        let app = self.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            let event_types = vec![
                DomainEventType::DownloadQueueItemUpserted,
                DomainEventType::DownloadQueueItemRemoved,
            ];
            let mut wake_rx = app.runtime.domain_event_broadcast.subscribe();
            let mut cursor = match app
                .services
                .events
                .domain_events
                .list(&DomainEventFilter {
                    event_types: Some(event_types.clone()),
                    limit: 1,
                    ..DomainEventFilter::default()
                })
                .await
            {
                Ok(events) => events.first().map(|event| event.sequence).unwrap_or(0),
                Err(error) => {
                    tracing::warn!(
                        "download queue subscription initial cursor load failed: {error}"
                    );
                    return;
                }
            };

            let initial_items = match app.list_download_queue(&actor, true, false).await {
                Ok(items) => items,
                Err(error) => {
                    tracing::warn!("download queue subscription initial load failed: {error}");
                    return;
                }
            };

            let mut items = initial_items
                .into_iter()
                .map(|item| (download_queue_projection_key(&item), item))
                .collect::<HashMap<_, _>>();

            loop {
                let batch = match app
                    .services
                    .events
                    .domain_events
                    .list(&DomainEventFilter {
                        event_types: Some(event_types.clone()),
                        after_sequence: Some(cursor),
                        limit: 100,
                        ..DomainEventFilter::default()
                    })
                    .await
                {
                    Ok(batch) => batch,
                    Err(error) => {
                        tracing::warn!("download queue subscription catch-up failed: {error}");
                        return;
                    }
                };
                if batch.is_empty() {
                    break;
                }

                let count = batch.len();
                for event in batch {
                    cursor = event.sequence;
                    apply_download_queue_projection_event(&mut items, &event);
                }
                if count < 100 {
                    break;
                }
            }

            let initial = sorted_download_queue_items(&items);
            if tx.send(initial).is_err() {
                return;
            }

            loop {
                let next_events = match app
                    .services
                    .events
                    .domain_events
                    .list(&DomainEventFilter {
                        event_types: Some(event_types.clone()),
                        after_sequence: Some(cursor),
                        limit: 100,
                        ..DomainEventFilter::default()
                    })
                    .await
                {
                    Ok(events) if !events.is_empty() => events,
                    Ok(_) => match wake_rx.recv().await {
                        Ok(sequence) => {
                            if sequence > cursor {
                                cursor = sequence.saturating_sub(1);
                            }
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::debug!(
                                "download queue subscription lagged, skipped {n} wakeups"
                            );
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    Err(error) => {
                        tracing::warn!("download queue subscription replay failed: {error}");
                        break;
                    }
                };

                for event in next_events {
                    cursor = event.sequence;
                    if let Some(snapshot) =
                        apply_download_queue_projection_event(&mut items, &event)
                        && tx.send(snapshot).is_err()
                    {
                        return;
                    }
                }
            }
        });
        Ok(rx)
    }

    pub async fn queue_manual_import(
        &self,
        actor: &User,
        title_id: Option<String>,
        client_type: String,
        download_client_item_id: String,
        files: Option<Vec<crate::ManualImportFileMapping>>,
    ) -> AppResult<String> {
        require(actor, &Entitlement::TriggerActions)?;

        let source_ref = download_client_item_id.trim().to_string();
        if source_ref.is_empty() {
            return Err(AppError::Validation(
                "download client item id is required".to_string(),
            ));
        }

        let normalized_client_type = client_type.trim().to_lowercase();
        if normalized_client_type.is_empty() {
            return Err(AppError::Validation("client type is required".to_string()));
        }

        let files = files.unwrap_or_default();
        if !files.is_empty() && title_id.is_none() {
            return Err(AppError::Validation(
                "title id is required for mapped manual import".to_string(),
            ));
        }

        if let Some(existing) = self
            .services
            .workflow
            .imports
            .get_import_by_source_ref_and_type(
                &normalized_client_type,
                &source_ref,
                ImportType::ManualImport,
            )
            .await?
            && existing.status.is_active()
        {
            return Ok(existing.id);
        }

        let payload_json = serde_json::to_string(&crate::ManualImportRequestPayload {
            requested_by_user_id: Some(actor.id.clone()),
            title_id: title_id.clone(),
            download_client_item_id: source_ref.clone(),
            client_type: normalized_client_type.clone(),
            files,
            requested_at: Utc::now().to_rfc3339(),
        })
        .map_err(|error| AppError::Repository(error.to_string()))?;

        let import_id = self
            .services
            .workflow
            .imports
            .queue_import_request(
                normalized_client_type.clone(),
                source_ref.clone(),
                ImportType::ManualImport.as_str().to_string(),
                payload_json,
            )
            .await?;

        let title = match title_id.as_deref() {
            Some(id) => self.services.catalog.titles.get_by_id(id).await?,
            None => None,
        };
        self.emit_import_requested_event(
            Some(actor.id.clone()),
            title.as_ref(),
            normalized_client_type,
            source_ref,
            scryer_domain::ImportRequestKind::Manual,
        )
        .await;

        Ok(import_id)
    }

    pub async fn trigger_manual_import(
        &self,
        actor: &User,
        completed: &CompletedDownload,
        override_title_id: Option<&str>,
    ) -> AppResult<scryer_domain::ImportResult> {
        require(actor, &Entitlement::TriggerActions)?;

        // If a title_id override is provided, inject it into the parameters
        let mut completed = completed.clone();
        if let Some(title_id) = override_title_id
            && !completed
                .parameters
                .iter()
                .any(|(k, _)| k == "*scryer_title_id")
        {
            completed
                .parameters
                .push(("*scryer_title_id".to_string(), title_id.to_string()));
        }

        crate::import_workflow::import_completed_download(self, actor, &completed).await
    }

    pub async fn ignore_tracked_download(
        &self,
        actor: &User,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        require(actor, &Entitlement::TriggerActions)?;
        let handle = self
            .runtime
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .ignore(crate::tracked_downloads::tracked_download_id(
                client_type,
                download_client_item_id,
            ))
            .await?;
        Ok(())
    }

    pub async fn mark_tracked_download_failed(
        &self,
        actor: &User,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        require(actor, &Entitlement::TriggerActions)?;
        let handle = self
            .runtime
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .mark_failed(crate::tracked_downloads::tracked_download_id(
                client_type,
                download_client_item_id,
            ))
            .await?;
        Ok(())
    }

    pub async fn retry_tracked_download_import(
        &self,
        actor: &User,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        require(actor, &Entitlement::TriggerActions)?;
        let handle = self
            .runtime
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .retry_import(crate::tracked_downloads::tracked_download_id(
                client_type,
                download_client_item_id,
            ))
            .await?;
        Ok(())
    }

    pub async fn assign_tracked_download_title(
        &self,
        actor: &User,
        client_type: &str,
        download_client_item_id: &str,
        title_id: &str,
    ) -> AppResult<()> {
        require(actor, &Entitlement::TriggerActions)?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.services
            .workflow
            .download_submissions
            .record_submission(DownloadSubmission {
                title_id: title.id.clone(),
                facet: title.facet.as_str().to_string(),
                download_client_type: client_type.to_string(),
                download_client_item_id: download_client_item_id.to_string(),
                source_title: Some(title.name.clone()),
                collection_id: None,
            })
            .await?;
        let handle = self
            .runtime
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .assign_title(
                crate::tracked_downloads::tracked_download_id(client_type, download_client_item_id),
                title.id,
            )
            .await?;
        Ok(())
    }

    pub async fn pause_download_queue_item(
        &self,
        actor: &User,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        require(actor, &Entitlement::TriggerActions)?;
        self.services
            .integrations
            .download_client
            .pause_queue_item(download_client_item_id)
            .await?;
        self.emit_download_queue_item_command_issued_event(
            Some(actor.id.clone()),
            download_client_item_id.to_string(),
            scryer_domain::DownloadQueueCommandAction::Pause,
        )
        .await;
        Ok(())
    }

    pub async fn resume_download_queue_item(
        &self,
        actor: &User,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        require(actor, &Entitlement::TriggerActions)?;
        self.services
            .integrations
            .download_client
            .resume_queue_item(download_client_item_id)
            .await?;
        self.emit_download_queue_item_command_issued_event(
            Some(actor.id.clone()),
            download_client_item_id.to_string(),
            scryer_domain::DownloadQueueCommandAction::Resume,
        )
        .await;
        Ok(())
    }

    pub async fn delete_download_queue_item(
        &self,
        actor: &User,
        download_client_item_id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        require(actor, &Entitlement::TriggerActions)?;
        self.services
            .integrations
            .download_client
            .delete_queue_item(download_client_item_id, is_history)
            .await?;
        self.emit_download_queue_item_command_issued_event(
            Some(actor.id.clone()),
            download_client_item_id.to_string(),
            scryer_domain::DownloadQueueCommandAction::Delete,
        )
        .await;
        Ok(())
    }

    pub async fn get_download_client_config(
        &self,
        actor: &User,
        client_id: &str,
    ) -> AppResult<Option<DownloadClientConfig>> {
        require(actor, &Entitlement::ManageConfig)?;
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err(AppError::Validation("client id is required".into()));
        }

        self.services
            .integrations
            .download_client_configs
            .get_by_id(client_id)
            .await
    }

    pub async fn create_download_client_config(
        &self,
        actor: &User,
        input: NewDownloadClientConfig,
    ) -> AppResult<DownloadClientConfig> {
        require(actor, &Entitlement::ManageConfig)?;

        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::Validation(
                "download client name is required".into(),
            ));
        }

        let client_type = self.normalize_download_client_type(input.client_type)?;
        let config_json = self.normalize_download_client_config_json(input.config_json)?;

        let existing = self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await?;
        let client_priority = existing
            .into_iter()
            .map(|entry| entry.client_priority)
            .max()
            .unwrap_or(0)
            + 1;

        let config = DownloadClientConfig {
            id: Id::new().0,
            name,
            client_type,
            config_json,
            client_priority,
            is_enabled: input.is_enabled,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let created = self
            .services
            .integrations
            .download_client_configs
            .create(config)
            .await?;
        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "download_client",
            Some(created.id.clone()),
            scryer_domain::ConfigurationChangeAction::Saved,
        )
        .await;

        Ok(created)
    }

    pub async fn update_download_client_config(
        &self,
        actor: &User,
        update: DownloadClientConfigUpdate,
    ) -> AppResult<DownloadClientConfig> {
        require(actor, &Entitlement::ManageConfig)?;
        let client_id = update.id.trim();
        if client_id.is_empty() {
            return Err(AppError::Validation("client id is required".into()));
        }

        if !update.has_changes() {
            return Err(AppError::Validation(
                "at least one download client field must be provided".into(),
            ));
        }

        let normalized_name = update.name.map(|value| value.trim().to_string());
        if normalized_name
            .as_ref()
            .is_some_and(|value| value.is_empty())
        {
            return Err(AppError::Validation("client name cannot be empty".into()));
        }

        let normalized_client_type = match update.client_type {
            Some(value) => Some(self.normalize_download_client_type(value)?),
            None => None,
        };
        let normalized_config_json = match update.config_json {
            Some(value) => Some(self.normalize_download_client_config_json(value)?),
            None => None,
        };

        let updated = self
            .services
            .integrations
            .download_client_configs
            .update(DownloadClientConfigUpdate {
                id: client_id.to_string(),
                name: normalized_name,
                client_type: normalized_client_type,
                config_json: normalized_config_json,
                is_enabled: update.is_enabled,
            })
            .await?;
        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "download_client",
            Some(updated.id.clone()),
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;

        Ok(updated)
    }

    pub async fn delete_download_client_config(
        &self,
        actor: &User,
        client_id: &str,
    ) -> AppResult<()> {
        require(actor, &Entitlement::ManageConfig)?;
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err(AppError::Validation("client id is required".into()));
        }

        self.services
            .integrations
            .download_client_configs
            .delete(client_id)
            .await?;
        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "download_client",
            Some(client_id.to_string()),
            scryer_domain::ConfigurationChangeAction::Deleted,
        )
        .await;

        Ok(())
    }

    pub async fn reorder_download_clients(
        &self,
        actor: &User,
        ordered_ids: Vec<String>,
    ) -> AppResult<()> {
        require(actor, &Entitlement::ManageConfig)?;
        self.services
            .integrations
            .download_client_configs
            .reorder(ordered_ids)
            .await
    }
}

pub async fn start_download_queue_poller(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
    mut command_rx: tokio::sync::mpsc::Receiver<crate::tracked_downloads::TrackedDownloadCommand>,
) {
    use crate::tracked_downloads::{TrackedDownloadService, tracked_download_id};
    use scryer_domain::TrackedDownloadState;

    let actor = match app.find_or_create_default_user().await {
        Ok(actor) => actor,
        Err(error) => {
            tracing::warn!(error = %error, "download queue poller failed to resolve actor");
            return;
        }
    };

    let mut tracker = TrackedDownloadService::new();
    let mut previous_items: HashMap<String, DownloadQueueItem> = HashMap::new();

    tracing::info!("download queue poller started (2s interval, tracked downloads enabled)");
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
    let mut commands_open = true;
    loop {
        tokio::select! {
            _ = token.cancelled() => {
                tracing::info!("download queue poller shutting down");
                break;
            }
            maybe_command = command_rx.recv(), if commands_open => {
                match maybe_command {
                    Some(command) => {
                        handle_tracked_download_command(&app, &mut tracker, command).await;
                    }
                    None => {
                        commands_open = false;
                    }
                }
            }
            _ = interval.tick() => {
                match app.collect_download_queue_items(true, false, false).await {
                    Ok(mut items) => {
                        let mut seen_ids = HashSet::new();
                        let completed_download_lookup =
                            crate::completed_download_handler::load_completed_download_lookup_for_items(
                                &app,
                                &items,
                            )
                            .await;

                        // Phase 1: Refresh — track each item and run checks.
                        for item in items.iter() {
                            let id = tracked_download_id(&item.client_type, &item.download_client_item_id);
                            seen_ids.insert(id.clone());

                            let is_new = tracker.find(&id).is_none();
                            tracker.track(&app, item.clone()).await;

                            if let Some(td) = tracker.find(&id)
                                && is_new
                            {
                                tracing::info!(
                                    id = %td.id,
                                    state = ?td.state,
                                    client_state = ?td.client_item.state,
                                    match_type = ?td.match_type,
                                    title_id = ?td.title_id,
                                    title_name = %td.client_item.title_name,
                                    "tracked: new download"
                                );
                            }

                            if let Some(td) = tracker.find_mut(&id)
                                && (td.state == TrackedDownloadState::Downloading
                                    || td.state == TrackedDownloadState::ImportBlocked)
                            {
                                let state_before = td.state;
                                crate::failed_download_handler::check(td);
                                crate::completed_download_handler::check_with_lookup(
                                    &app,
                                    td,
                                    completed_download_lookup.as_ref(),
                                )
                                .await;
                                if td.state != state_before {
                                    tracing::info!(
                                        id = %id,
                                        from = ?state_before,
                                        to = ?td.state,
                                        "tracked: state transition after check"
                                    );
                                }
                            }
                        }

                        tracker.update_trackable(&seen_ids);

                        // Phase 2: Process — import pending and failed items.
                        let trackable_ids = tracker.get_trackable_ids();

                        for id in &trackable_ids {
                            let mut terminal_state_to_persist = None;

                            if let Some(td) = tracker.find_mut(id) {
                                if td.state == TrackedDownloadState::ImportPending {
                                    let transitioned_terminal =
                                        crate::completed_download_handler::import(&app, &actor, td)
                                            .await;
                                    if transitioned_terminal {
                                        terminal_state_to_persist = Some(td.state);
                                    }
                                }

                                if td.state == TrackedDownloadState::FailedPending {
                                    crate::failed_download_handler::process_failed(&app, td).await;
                                    terminal_state_to_persist = Some(TrackedDownloadState::Failed);
                                }
                            }

                            if let Some(state) = terminal_state_to_persist {
                                tracing::info!(
                                    id = %id,
                                    state = state.as_str(),
                                    "tracked: persisting terminal state"
                                );
                                let persisted = tracker.persist_terminal_state(&app, id, state).await;
                                if persisted {
                                    if let Some(td) = tracker.find(id) {
                                        try_remove_from_client(&app, td, state).await;
                                    }
                                    tracker.stop_tracking(id);
                                }
                            }
                        }

                        // Enrich items with tracked state before broadcasting.
                        for item in &mut items {
                            let id = tracked_download_id(&item.client_type, &item.download_client_item_id);
                            if let Some(td) = tracker.find(&id) {
                                let metadata = tracked_download_queue_snapshot(td);
                                apply_tracked_download_queue_metadata(item, &metadata);
                            }
                        }

                        // Emit download queue gauge by state.
                        let mut counts = [0u64; 9];
                        for item in &items {
                            match item.state {
                                scryer_domain::DownloadQueueState::Queued => counts[0] += 1,
                                scryer_domain::DownloadQueueState::Downloading => counts[1] += 1,
                                scryer_domain::DownloadQueueState::Paused => counts[2] += 1,
                                scryer_domain::DownloadQueueState::Completed => counts[3] += 1,
                                scryer_domain::DownloadQueueState::ImportPending => counts[4] += 1,
                                scryer_domain::DownloadQueueState::Failed => counts[5] += 1,
                                scryer_domain::DownloadQueueState::Verifying => counts[6] += 1,
                                scryer_domain::DownloadQueueState::Repairing => counts[7] += 1,
                                scryer_domain::DownloadQueueState::Extracting => counts[8] += 1,
                            }
                        }
                        let labels = ["queued", "downloading", "paused", "completed", "import_pending", "failed", "verifying", "repairing", "extracting"];
                        for (label, &count) in labels.iter().zip(&counts) {
                            metrics::gauge!("scryer_download_queue_items", "state" => *label).set(count as f64);
                        }

                        publish_download_queue_snapshot_events(
                            &app,
                            Some(actor.id.clone()),
                            &mut previous_items,
                            &items,
                        )
                        .await;
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "download queue poll failed");
                    }
                }
            }
        }
    }
}

async fn handle_tracked_download_command(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    command: crate::tracked_downloads::TrackedDownloadCommand,
) {
    use crate::tracked_downloads::TrackedDownloadCommand;
    use scryer_domain::{TrackedDownloadState, TrackedDownloadStatus};

    match command {
        TrackedDownloadCommand::MarkImported { id, reply } => {
            let result = if let Some(td) = tracker.find_mut(&id) {
                td.state = TrackedDownloadState::Imported;
                td.status = TrackedDownloadStatus::Ok;
                td.status_messages.clear();
                tracker
                    .persist_terminal_state(app, &id, TrackedDownloadState::Imported)
                    .await;
                if let Some(td) = tracker.find(&id) {
                    try_remove_from_client(app, td, TrackedDownloadState::Imported).await;
                }
                tracker.stop_tracking(&id);
                Ok(())
            } else {
                Err(AppError::NotFound(format!("tracked download {id}")))
            };
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::Ignore { id, reply } => {
            let result = if let Some(td) = tracker.find_mut(&id) {
                td.state = TrackedDownloadState::Ignored;
                td.status = TrackedDownloadStatus::Ok;
                td.status_messages.clear();
                tracker
                    .persist_terminal_state(app, &id, TrackedDownloadState::Ignored)
                    .await;
                if let Some(td) = tracker.find(&id) {
                    try_remove_from_client(app, td, TrackedDownloadState::Ignored).await;
                }
                tracker.stop_tracking(&id);
                Ok(())
            } else {
                Err(AppError::NotFound(format!("tracked download {id}")))
            };
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::MarkFailed { id, reply } => {
            let result = if let Some(td) = tracker.find_mut(&id) {
                td.state = TrackedDownloadState::FailedPending;
                crate::failed_download_handler::process_failed(app, td).await;
                tracker
                    .persist_terminal_state(app, &id, TrackedDownloadState::Failed)
                    .await;
                if let Some(td) = tracker.find(&id) {
                    try_remove_from_client(app, td, TrackedDownloadState::Failed).await;
                }
                tracker.stop_tracking(&id);
                Ok(())
            } else {
                Err(AppError::NotFound(format!("tracked download {id}")))
            };
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::RetryImport { id, reply } => {
            let result = if let Some(td) = tracker.find_mut(&id) {
                td.state = TrackedDownloadState::ImportPending;
                td.status = TrackedDownloadStatus::Ok;
                td.status_messages.clear();
                td.import_attempted = false;
                td.path_missing_since = None;
                Ok(())
            } else {
                Err(AppError::NotFound(format!("tracked download {id}")))
            };
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::AssignTitle {
            id,
            title_id,
            reply,
        } => {
            let title = match app.services.catalog.titles.get_by_id(&title_id).await {
                Ok(Some(title)) => title,
                Ok(None) => {
                    let _ = reply.send(Err(AppError::NotFound(format!("title {title_id}"))));
                    return;
                }
                Err(error) => {
                    let _ = reply.send(Err(error));
                    return;
                }
            };

            let result = if let Some(td) = tracker.find_mut(&id) {
                crate::tracked_downloads::assign_title_to_tracked_download(app, td, &title).await;
                Ok(())
            } else {
                Err(AppError::NotFound(format!("tracked download {id}")))
            };
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::Snapshot { ids, reply } => {
            let snapshot = ids
                .into_iter()
                .filter_map(|id| {
                    tracker
                        .find(&id)
                        .map(|tracked| (id, tracked_download_queue_snapshot(tracked)))
                })
                .collect();
            let _ = reply.send(snapshot);
        }
    }
}

/// Remove a download from the client after reaching a terminal state,
/// if the client's config has `remove_completed` or `remove_failed` enabled.
async fn try_remove_from_client(
    app: &AppUseCase,
    td: &crate::tracked_downloads::TrackedDownload,
    state: scryer_domain::TrackedDownloadState,
) {
    // Look up the client config to check removal settings.
    let config = match app
        .services
        .integrations
        .download_client_configs
        .list(Some(td.client_type.clone()))
        .await
    {
        Ok(configs) => configs.into_iter().next(),
        Err(_) => None,
    };

    let should_remove = if let Some(config) = config {
        let parsed: serde_json::Value =
            serde_json::from_str(&config.config_json).unwrap_or_default();
        match state {
            scryer_domain::TrackedDownloadState::Imported => parsed
                .get("remove_completed")
                .or_else(|| parsed.get("removeCompleted"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            scryer_domain::TrackedDownloadState::Failed => parsed
                .get("remove_failed")
                .or_else(|| parsed.get("removeFailed"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            scryer_domain::TrackedDownloadState::Ignored => {
                // Always remove ignored downloads — the user explicitly dismissed them.
                true
            }
            _ => false,
        }
    } else {
        false
    };

    if !should_remove {
        return;
    }

    let item_id = &td.client_item.download_client_item_id;
    // Completed/failed items are in the client's history, not the active queue.
    let is_history = matches!(
        state,
        scryer_domain::TrackedDownloadState::Imported
            | scryer_domain::TrackedDownloadState::Failed
            | scryer_domain::TrackedDownloadState::Ignored
    );

    tracing::info!(
        id = %td.id,
        item_id,
        state = state.as_str(),
        is_history,
        "removing download from client"
    );

    if let Err(error) = app
        .services
        .integrations
        .download_client
        .delete_queue_item(item_id, is_history)
        .await
    {
        tracing::warn!(
            error = %error,
            id = %td.id,
            item_id,
            "failed to remove download from client"
        );
    }
}

fn parse_sort_value(left: Option<&str>, right: Option<&str>) -> std::cmp::Ordering {
    fn parse(value: Option<&str>) -> i64 {
        value
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0)
    }

    parse(left).cmp(&parse(right))
}

fn dedupe_download_queue_items(items: Vec<DownloadQueueItem>) -> Vec<DownloadQueueItem> {
    let mut deduped: Vec<DownloadQueueItem> = Vec::with_capacity(items.len());
    let mut key_to_index: HashMap<String, usize> = HashMap::with_capacity(items.len());

    for item in items {
        let key = download_queue_item_key(&item);
        if let Some(index) = key_to_index.get(&key).copied() {
            merge_download_queue_item(&mut deduped[index], item);
            continue;
        }

        key_to_index.insert(key, deduped.len());
        deduped.push(item);
    }

    deduped
}

fn download_queue_item_key(item: &DownloadQueueItem) -> String {
    if item.client_type.is_empty() && item.download_client_item_id.is_empty() {
        return item.id.clone();
    }

    format!("{}:{}", item.client_type, item.download_client_item_id)
}

fn merge_download_queue_item(existing: &mut DownloadQueueItem, incoming: DownloadQueueItem) {
    if existing.title_id.is_none() {
        existing.title_id = incoming.title_id.clone();
    }
    if existing.title_name.trim().is_empty() || existing.title_name == "Unnamed download" {
        existing.title_name = incoming.title_name.clone();
    }
    if existing.facet.is_none() {
        existing.facet = incoming.facet.clone();
    }
    if existing.client_id.is_empty() {
        existing.client_id = incoming.client_id.clone();
    }
    if existing.client_name.is_empty() {
        existing.client_name = incoming.client_name.clone();
    }
    if existing.client_type.is_empty() {
        existing.client_type = incoming.client_type.clone();
    }

    if let Some(size_bytes) = incoming.size_bytes {
        existing.size_bytes = Some(existing.size_bytes.unwrap_or(size_bytes).max(size_bytes));
    }
    if existing.remaining_seconds.is_none() {
        existing.remaining_seconds = incoming.remaining_seconds;
    }
    if existing.queued_at.is_none() {
        existing.queued_at = incoming.queued_at.clone();
    }
    if existing.last_updated_at.is_none() {
        existing.last_updated_at = incoming.last_updated_at.clone();
    }

    if queue_state_merge_rank(&incoming.state) > queue_state_merge_rank(&existing.state)
        || (incoming.progress_percent > existing.progress_percent
            && queue_state_merge_rank(&incoming.state) == queue_state_merge_rank(&existing.state))
    {
        existing.state = incoming.state;
        existing.progress_percent = incoming.progress_percent;
    } else {
        existing.progress_percent = existing.progress_percent.max(incoming.progress_percent);
    }

    existing.attention_required |= incoming.attention_required;
    if existing.attention_reason.is_none() {
        existing.attention_reason = incoming.attention_reason.clone();
    }
    if incoming.import_status.is_some() {
        existing.import_status = incoming.import_status;
    }
    if incoming.import_error_message.is_some() {
        existing.import_error_message = incoming.import_error_message.clone();
    }
    if incoming.imported_at.is_some() {
        existing.imported_at = incoming.imported_at.clone();
    }
    existing.is_scryer_origin |= incoming.is_scryer_origin;
}

fn queue_state_merge_rank(state: &DownloadQueueState) -> u8 {
    match state {
        DownloadQueueState::Paused => 0,
        DownloadQueueState::Queued => 1,
        DownloadQueueState::Downloading => 2,
        DownloadQueueState::Verifying
        | DownloadQueueState::Repairing
        | DownloadQueueState::Extracting => 3,
        DownloadQueueState::Completed => 4,
        DownloadQueueState::ImportPending => 5,
        DownloadQueueState::Failed => 6,
    }
}

fn is_active_download_state(state: &DownloadQueueState) -> bool {
    matches!(
        state,
        DownloadQueueState::Downloading
            | DownloadQueueState::Queued
            | DownloadQueueState::Paused
            | DownloadQueueState::Verifying
            | DownloadQueueState::Repairing
            | DownloadQueueState::Extracting
    )
}

fn is_history_download_state(state: &DownloadQueueState) -> bool {
    matches!(
        state,
        DownloadQueueState::Completed
            | DownloadQueueState::ImportPending
            | DownloadQueueState::Failed
    )
}

fn queue_state_sort_rank(state: &DownloadQueueState) -> u8 {
    match state {
        DownloadQueueState::Downloading => 0,
        DownloadQueueState::Verifying
        | DownloadQueueState::Repairing
        | DownloadQueueState::Extracting => 0,
        DownloadQueueState::Queued => 1,
        DownloadQueueState::Paused => 2,
        DownloadQueueState::ImportPending => 3,
        DownloadQueueState::Completed => 3,
        DownloadQueueState::Failed => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_tracked_download_queue_metadata, dedupe_download_queue_items,
        tracked_download_queue_snapshot,
    };
    use chrono::Utc;
    use scryer_domain::{
        DownloadQueueItem, DownloadQueueState, TitleMatchType, TrackedDownloadState,
        TrackedDownloadStatus,
    };

    fn item(id: &str, state: DownloadQueueState) -> DownloadQueueItem {
        DownloadQueueItem {
            id: id.to_string(),
            title_id: None,
            title_name: "Example".to_string(),
            facet: None,
            client_id: "client-1".to_string(),
            client_name: "Weaver".to_string(),
            client_type: "weaver".to_string(),
            state,
            progress_percent: 100,
            size_bytes: Some(100),
            remaining_seconds: None,
            queued_at: Some(Utc::now().timestamp_millis().to_string()),
            last_updated_at: Some(Utc::now().timestamp_millis().to_string()),
            attention_required: false,
            attention_reason: None,
            download_client_item_id: id.to_string(),
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        }
    }

    #[test]
    fn dedupe_download_queue_items_merges_duplicate_client_job_ids() {
        let mut first = item("job-1", DownloadQueueState::Completed);
        first.import_error_message = Some("failed to import".to_string());
        let mut second = item("job-1", DownloadQueueState::Completed);
        second.title_id = Some("title-1".to_string());

        let deduped = dedupe_download_queue_items(vec![
            first,
            second,
            item("job-2", DownloadQueueState::Queued),
        ]);

        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].download_client_item_id, "job-1");
        assert_eq!(deduped[0].title_id.as_deref(), Some("title-1"));
        assert_eq!(
            deduped[0].import_error_message.as_deref(),
            Some("failed to import")
        );
    }

    #[test]
    fn apply_tracked_download_queue_metadata_backfills_missing_facet() {
        let mut queue_item = item("job-1", DownloadQueueState::Completed);
        let tracked = crate::tracked_downloads::TrackedDownload {
            id: "nzbget:job-1".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item: queue_item.clone(),
            state: TrackedDownloadState::ImportBlocked,
            status: TrackedDownloadStatus::Warning,
            status_messages: vec!["needs manual import".to_string()],
            title_id: Some("title-1".to_string()),
            facet: Some("series".to_string()),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::TitleParse,
            is_trackable: true,
            import_attempted: false,
            path_missing_since: None,
        };
        let metadata = tracked_download_queue_snapshot(&tracked);

        apply_tracked_download_queue_metadata(&mut queue_item, &metadata);

        assert_eq!(queue_item.title_id.as_deref(), Some("title-1"));
        assert_eq!(queue_item.facet.as_deref(), Some("series"));
        assert_eq!(
            queue_item.tracked_state,
            Some(TrackedDownloadState::ImportBlocked)
        );
        assert_eq!(
            queue_item.tracked_status,
            Some(TrackedDownloadStatus::Warning)
        );
        assert_eq!(
            queue_item.tracked_match_type,
            Some(TitleMatchType::TitleParse)
        );
    }
}
