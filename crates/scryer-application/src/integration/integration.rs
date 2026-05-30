use super::*;
use crate::domain_events::new_download_queue_domain_event;
use crate::event_views::{
    apply_download_queue_projection_event, sort_download_queue_items, sorted_download_queue_items,
};
use crate::tracked_downloads::{
    TrackedDownload, TrackedDownloadQueueMetadata, publish_runtime_tracked_download_snapshot_cache,
    tracked_download_id,
};
use crate::types::DownloadClientFilterOption;
use scryer_domain::{
    CompletedDownload, DomainEventFilter, DomainEventPayload, DomainEventType,
    DownloadQueueDeleteStatus, DownloadQueueItemRemovedEventData,
    DownloadQueueItemUpsertedEventData, ImportType, TrackedDownloadState, TrackedDownloadStatus,
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

const DOWNLOAD_QUEUE_RECENT_ACTIVITY_LIMIT: usize = 100;
const DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT: usize = 100;
const TRACKED_DOWNLOAD_SNAPSHOT_READ_BUDGET: Duration = Duration::from_millis(25);
const TRACKED_DOWNLOAD_BACKGROUND_WORKER_LIMIT: usize = 1;
const MANAGED_INDEXER_SCOPE_IDS: &[&str] = &["movie", "series", "anime"];

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrackedDownloadBackgroundWorkKind {
    Import,
    Failed,
}

impl TrackedDownloadBackgroundWorkKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug)]
struct TrackedDownloadBackgroundWorkResult {
    id: String,
    kind: TrackedDownloadBackgroundWorkKind,
    outcome: Result<TrackedDownload, String>,
    elapsed: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DownloadQueueBucket {
    Activity,
    Import,
    HistorySuccess,
    HistoryFailed,
}

#[derive(Clone, Debug)]
pub(crate) enum ManualImportSourceResolution {
    Eligible {
        completed: Option<CompletedDownload>,
    },
    SourceFailed {
        message: String,
    },
    NotEligible {
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ClassifiedDownloadQueueItem {
    display_state: DownloadDisplayState,
    bucket: DownloadQueueBucket,
    activity_filter: Option<DownloadActivityFilter>,
    import_filter: Option<DownloadImportFilter>,
    history_filter: Option<DownloadHistoryFilter>,
}

fn push_queue_status_detail(
    values: &mut Vec<String>,
    seen: &mut HashSet<String>,
    raw: Option<&str>,
) {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    if seen.insert(value.to_string()) {
        values.push(value.to_string());
    }
}

fn build_download_queue_status_detail(item: &DownloadQueueItem) -> String {
    let mut values = Vec::new();
    let mut seen = HashSet::new();
    for message in &item.tracked_status_messages {
        push_queue_status_detail(&mut values, &mut seen, Some(message));
    }
    push_queue_status_detail(&mut values, &mut seen, item.attention_reason.as_deref());
    push_queue_status_detail(&mut values, &mut seen, item.delete_error_message.as_deref());
    push_queue_status_detail(&mut values, &mut seen, item.import_error_message.as_deref());
    values.join("\n")
}

fn is_post_processing_reason(reason: Option<&str>) -> bool {
    let Some(reason) = reason else {
        return false;
    };
    let normalized = reason.trim().to_ascii_uppercase();
    normalized.contains("PP_QUEUED")
        || normalized.contains("POSTPROCESSING")
        || normalized.contains("UNPACKING")
        || normalized.contains("REPAIRING")
        || normalized.contains("VERIFYING")
        || normalized.contains("RENAMING")
        || normalized.contains("MOVING")
        || normalized.contains("EXECUTING_SCRIPT")
}

fn base_download_queue_display_state(item: &DownloadQueueItem) -> DownloadDisplayState {
    if item.state == DownloadQueueState::Failed {
        return DownloadDisplayState::Failed;
    }

    match item.import_status {
        Some(ImportStatus::Pending | ImportStatus::Running | ImportStatus::Processing) => {
            return DownloadDisplayState::Importing;
        }
        Some(ImportStatus::Failed | ImportStatus::Skipped)
            if matches!(
                item.tracked_state,
                Some(TrackedDownloadState::ImportBlocked)
            ) || matches!(
                item.state,
                DownloadQueueState::Completed
                    | DownloadQueueState::ImportPending
                    | DownloadQueueState::Failed
            ) =>
        {
            return DownloadDisplayState::ImportFailed;
        }
        _ => {}
    }

    match item.tracked_state {
        Some(TrackedDownloadState::ImportBlocked) => return DownloadDisplayState::ImportBlocked,
        Some(TrackedDownloadState::ImportPending) => return DownloadDisplayState::ImportPending,
        _ => {}
    }

    let failure_reason = build_download_queue_status_detail(item);
    let can_derive_blocked_state = item.tracked_state.is_none()
        && !failure_reason.is_empty()
        && matches!(
            item.state,
            DownloadQueueState::Completed | DownloadQueueState::ImportPending
        )
        && matches!(
            item.import_status,
            Some(ImportStatus::Skipped | ImportStatus::Failed)
        );
    if can_derive_blocked_state {
        return DownloadDisplayState::ImportBlocked;
    }

    match item.state {
        DownloadQueueState::Queued => DownloadDisplayState::Queued,
        DownloadQueueState::Downloading => {
            if is_post_processing_reason(item.attention_reason.as_deref()) {
                DownloadDisplayState::PostProcessing
            } else {
                DownloadDisplayState::Downloading
            }
        }
        DownloadQueueState::Verifying
        | DownloadQueueState::Repairing
        | DownloadQueueState::Extracting => DownloadDisplayState::PostProcessing,
        DownloadQueueState::Paused => DownloadDisplayState::Paused,
        DownloadQueueState::Completed => DownloadDisplayState::Completed,
        DownloadQueueState::ImportPending => DownloadDisplayState::ImportPending,
        DownloadQueueState::Failed => DownloadDisplayState::Failed,
    }
}

fn bucket_for_base_display_state(state: DownloadDisplayState) -> DownloadQueueBucket {
    match state {
        DownloadDisplayState::Queued
        | DownloadDisplayState::Downloading
        | DownloadDisplayState::Paused
        | DownloadDisplayState::PostProcessing => DownloadQueueBucket::Activity,
        DownloadDisplayState::Importing
        | DownloadDisplayState::ImportPending
        | DownloadDisplayState::ImportBlocked
        | DownloadDisplayState::ImportFailed => DownloadQueueBucket::Import,
        DownloadDisplayState::Completed => DownloadQueueBucket::HistorySuccess,
        DownloadDisplayState::Failed => DownloadQueueBucket::HistoryFailed,
        DownloadDisplayState::Removing | DownloadDisplayState::RemoveFailed => {
            DownloadQueueBucket::HistoryFailed
        }
    }
}

fn normalize_routing_categories(categories: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for category in categories {
        let category = category.trim().to_string();
        if category.is_empty() || !seen.insert(category.clone()) {
            continue;
        }
        normalized.push(category);
    }
    normalized
}

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

pub fn derive_download_queue_display_state(item: &DownloadQueueItem) -> DownloadDisplayState {
    let base_state = base_download_queue_display_state(item);
    match item.delete_status {
        Some(DownloadQueueDeleteStatus::Queued | DownloadQueueDeleteStatus::Running) => {
            DownloadDisplayState::Removing
        }
        Some(DownloadQueueDeleteStatus::Failed) => DownloadDisplayState::RemoveFailed,
        _ => base_state,
    }
}

fn classify_download_queue_item(item: &DownloadQueueItem) -> ClassifiedDownloadQueueItem {
    let base_state = base_download_queue_display_state(item);
    let base_bucket = bucket_for_base_display_state(base_state);
    let display_state = derive_download_queue_display_state(item);

    let bucket = match (base_bucket, display_state) {
        (DownloadQueueBucket::Import, DownloadDisplayState::RemoveFailed)
        | (DownloadQueueBucket::Activity, DownloadDisplayState::RemoveFailed) => base_bucket,
        (_, DownloadDisplayState::RemoveFailed) => DownloadQueueBucket::HistoryFailed,
        _ => base_bucket,
    };

    let activity_filter = match base_state {
        DownloadDisplayState::Downloading => Some(DownloadActivityFilter::Downloading),
        DownloadDisplayState::Queued => Some(DownloadActivityFilter::Queued),
        DownloadDisplayState::Paused => Some(DownloadActivityFilter::Paused),
        DownloadDisplayState::PostProcessing => Some(DownloadActivityFilter::PostProcessing),
        _ => None,
    };

    let import_filter = match base_state {
        DownloadDisplayState::Importing => Some(DownloadImportFilter::Importing),
        DownloadDisplayState::ImportPending => Some(DownloadImportFilter::Pending),
        DownloadDisplayState::ImportBlocked => Some(DownloadImportFilter::Blocked),
        DownloadDisplayState::ImportFailed => Some(DownloadImportFilter::Failed),
        _ => None,
    };

    let history_filter = match bucket {
        DownloadQueueBucket::HistorySuccess => Some(DownloadHistoryFilter::Success),
        DownloadQueueBucket::HistoryFailed => Some(DownloadHistoryFilter::Failed),
        _ => None,
    };

    ClassifiedDownloadQueueItem {
        display_state,
        bucket,
        activity_filter,
        import_filter,
        history_filter,
    }
}

pub fn matches_download_activity_filter(
    item: &DownloadQueueItem,
    filter: DownloadActivityFilter,
) -> bool {
    let classified = classify_download_queue_item(item);
    if classified.bucket != DownloadQueueBucket::Activity {
        return false;
    }

    match filter {
        DownloadActivityFilter::All => true,
        _ => classified.activity_filter == Some(filter),
    }
}

pub fn matches_download_queue_filter(
    item: &DownloadQueueItem,
    include_history_only: bool,
    include_import_activity: bool,
    activity_filter: DownloadActivityFilter,
) -> bool {
    let classified = classify_download_queue_item(item);

    if include_history_only {
        return matches!(
            classified.bucket,
            DownloadQueueBucket::HistorySuccess | DownloadQueueBucket::HistoryFailed
        );
    }

    match classified.bucket {
        DownloadQueueBucket::Activity => match activity_filter {
            DownloadActivityFilter::All => true,
            _ => classified.activity_filter == Some(activity_filter),
        },
        DownloadQueueBucket::Import => {
            include_import_activity
                && matches!(
                    classified.import_filter,
                    Some(DownloadImportFilter::Importing | DownloadImportFilter::Pending)
                )
        }
        DownloadQueueBucket::HistorySuccess | DownloadQueueBucket::HistoryFailed => false,
    }
}

fn matches_download_import_filter(item: &DownloadQueueItem, filter: DownloadImportFilter) -> bool {
    let classified = classify_download_queue_item(item);
    if classified.bucket != DownloadQueueBucket::Import {
        return false;
    }

    match filter {
        DownloadImportFilter::All => true,
        _ => classified.import_filter == Some(filter),
    }
}

fn matches_download_history_filters(
    item: &DownloadQueueItem,
    filters: Option<&[DownloadHistoryFilter]>,
) -> bool {
    let classified = classify_download_queue_item(item);
    if !matches!(
        classified.bucket,
        DownloadQueueBucket::HistorySuccess | DownloadQueueBucket::HistoryFailed
    ) {
        return false;
    }

    match filters {
        None => true,
        Some([]) => false,
        Some(filters) if filters.contains(&DownloadHistoryFilter::All) => true,
        Some(filters) => classified
            .history_filter
            .is_some_and(|filter| filters.contains(&filter)),
    }
}

fn compare_case_insensitive(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase())
}

fn download_history_title(item: &DownloadQueueItem) -> &str {
    let title = item.title_name.trim();
    if title.is_empty() {
        item.download_client_item_id.as_str()
    } else {
        title
    }
}

fn download_history_client_label(item: &DownloadQueueItem) -> &str {
    let client_name = item.client_name.trim();
    if client_name.is_empty() {
        item.client_type.as_str()
    } else {
        client_name
    }
}

fn download_history_status_rank(item: &DownloadQueueItem) -> u8 {
    match classify_download_queue_item(item).bucket {
        DownloadQueueBucket::HistorySuccess => 0,
        DownloadQueueBucket::HistoryFailed => 1,
        _ => u8::MAX,
    }
}

fn compare_download_history_items(
    left: &DownloadQueueItem,
    right: &DownloadQueueItem,
    sort: DownloadHistorySort,
) -> std::cmp::Ordering {
    let ordering = match sort.key {
        DownloadHistorySortKey::Title => {
            compare_case_insensitive(download_history_title(left), download_history_title(right))
        }
        DownloadHistorySortKey::Client => compare_case_insensitive(
            download_history_client_label(left),
            download_history_client_label(right),
        )
        .then_with(|| compare_case_insensitive(&left.client_type, &right.client_type)),
        DownloadHistorySortKey::Status => {
            download_history_status_rank(left).cmp(&download_history_status_rank(right))
        }
        DownloadHistorySortKey::Progress => left.progress_percent.cmp(&right.progress_percent),
        DownloadHistorySortKey::Size => left
            .size_bytes
            .unwrap_or(0)
            .cmp(&right.size_bytes.unwrap_or(0)),
    };

    let ordering = match sort.direction {
        SortDirection::Asc => ordering,
        SortDirection::Desc => ordering.reverse(),
    };

    ordering
        .then_with(|| {
            parse_sort_value(
                right.last_updated_at.as_deref(),
                left.last_updated_at.as_deref(),
            )
        })
        .then_with(|| {
            compare_case_insensitive(download_history_title(left), download_history_title(right))
        })
        .then_with(|| {
            compare_case_insensitive(
                &left.download_client_item_id,
                &right.download_client_item_id,
            )
        })
}

fn sort_download_history_items(items: &mut [DownloadQueueItem], sort: DownloadHistorySort) {
    items.sort_by(|left, right| compare_download_history_items(left, right, sort));
}

fn download_queue_client_filter_key(item: &DownloadQueueItem) -> String {
    let client_id = item.client_id.trim();
    if !client_id.is_empty() {
        return client_id.to_string();
    }

    let client_type = item.client_type.trim();
    if !client_type.is_empty() {
        return client_type.to_ascii_lowercase();
    }

    item.id.clone()
}

fn collect_download_client_filter_options(
    items: &[DownloadQueueItem],
) -> Vec<DownloadClientFilterOption> {
    let mut seen = HashSet::new();
    let mut clients = Vec::new();

    for item in items {
        let client_id = download_queue_client_filter_key(item);
        if !seen.insert(client_id.clone()) {
            continue;
        }

        let client_name = item.client_name.trim();
        let client_type = item.client_type.trim();
        clients.push(DownloadClientFilterOption {
            client_id,
            client_name: if client_name.is_empty() {
                client_type.to_string()
            } else {
                client_name.to_string()
            },
            client_type: client_type.to_string(),
        });
    }

    clients.sort_by(|left, right| {
        left.client_name
            .to_ascii_lowercase()
            .cmp(&right.client_name.to_ascii_lowercase())
            .then_with(|| {
                left.client_type
                    .to_ascii_lowercase()
                    .cmp(&right.client_type.to_ascii_lowercase())
            })
            .then_with(|| left.client_id.cmp(&right.client_id))
    });
    clients
}

fn matches_download_history_client_ids(
    item: &DownloadQueueItem,
    client_ids: Option<&HashSet<String>>,
) -> bool {
    match client_ids {
        None => true,
        Some(ids) if ids.is_empty() => false,
        Some(ids) => ids.contains(&download_queue_client_filter_key(item)),
    }
}

fn annotate_download_queue_item(
    mut item: DownloadQueueItem,
    primary_client: Option<&DownloadClientConfig>,
) -> DownloadQueueItem {
    if let Some(primary_client) = primary_client {
        if item.client_id.is_empty() {
            item.client_id = primary_client.id.clone();
        }
        if item.client_name.is_empty() {
            item.client_name = primary_client.name.clone();
        }
        if item.client_type.is_empty() {
            item.client_type = primary_client.client_type.clone();
        }
    }
    item.attention_required = matches!(
        classify_download_queue_item(&item).bucket,
        DownloadQueueBucket::Import | DownloadQueueBucket::HistoryFailed
    );
    if item.attention_reason.is_none() {
        item.attention_reason = if item.attention_required {
            Some("requires attention".to_string())
        } else {
            None
        };
    }
    item
}

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

fn apply_delete_command_to_queue_item(
    item: &mut DownloadQueueItem,
    command: &crate::DownloadQueueCommandRecord,
) {
    item.delete_status = Some(command.status);
    item.delete_error_message = command.error_text.clone();
    if let Some(error_text) = command.error_text.as_ref() {
        item.attention_reason = Some(error_text.clone());
    }
}

fn queue_item_import_state_eligible(item: &DownloadQueueItem) -> bool {
    matches!(
        item.state,
        DownloadQueueState::Completed | DownloadQueueState::ImportPending
    )
}

fn source_identity_matches(
    item_client_id: &str,
    item_client_type: &str,
    item_id: &str,
    client_id: Option<&str>,
    client_type: Option<&str>,
    download_client_item_id: &str,
) -> bool {
    if item_id != download_client_item_id {
        return false;
    }

    let requested_client_id = client_id.map(str::trim).filter(|value| !value.is_empty());
    if requested_client_id.is_some_and(|client_id| item_client_id != client_id) {
        return false;
    }

    let requested_client_type = client_type.map(str::trim).filter(|value| !value.is_empty());
    requested_client_type
        .is_none_or(|client_type| item_client_type.eq_ignore_ascii_case(client_type))
}

fn source_failed_message(item: &DownloadQueueItem) -> String {
    let message = build_download_queue_status_detail(item).trim().to_string();
    if message.is_empty() {
        "source download failed before import".to_string()
    } else {
        message
    }
}

fn normalized_download_client_id(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("")
        .to_string()
}

fn download_queue_identity_key(
    client_id: Option<&str>,
    client_type: &str,
    download_client_item_id: &str,
) -> (String, String, String) {
    (
        normalized_download_client_id(client_id),
        client_type.to_string(),
        download_client_item_id.to_string(),
    )
}

pub async fn enrich_download_queue_items_from_submissions(
    app: &AppUseCase,
    items: &mut [DownloadQueueItem],
) {
    let client_items = items
        .iter()
        .map(|item| {
            DownloadSourceIdentity::new(
                Some(item.client_id.as_str()).filter(|value| !value.trim().is_empty()),
                &item.client_type,
                &item.download_client_item_id,
            )
        })
        .filter(|identity| !identity.client_type.is_empty() && !identity.item_id.is_empty())
        .collect::<Vec<_>>();

    if client_items.is_empty() {
        return;
    }

    let submissions = match app
        .services
        .workflow
        .download_submissions
        .list_for_client_items(&client_items)
        .await
    {
        Ok(submissions) => submissions,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to batch-load download submissions for queue enrichment"
            );
            return;
        }
    };

    let submission_map = submissions
        .into_iter()
        .filter(|submission| !submission.title_id.trim().is_empty())
        .map(|submission| {
            (
                download_queue_identity_key(
                    submission.download_client_id.as_deref(),
                    &submission.download_client_type,
                    &submission.download_client_item_id,
                ),
                submission,
            )
        })
        .collect::<HashMap<_, _>>();

    for item in items {
        let exact_key = download_queue_identity_key(
            Some(item.client_id.as_str()),
            &item.client_type,
            &item.download_client_item_id,
        );
        let legacy_submission = || {
            item.client_id.trim().is_empty().then(|| {
                submission_map.get(&download_queue_identity_key(
                    None,
                    &item.client_type,
                    &item.download_client_item_id,
                ))
            })?
        };
        if let Some(submission) = submission_map.get(&exact_key).or_else(legacy_submission) {
            item.is_scryer_origin = true;
            if item.title_id.is_none() {
                item.title_id = Some(submission.title_id.clone());
            }
            if item.episode_id.is_none() {
                item.episode_id = submission.scope.episode_id().map(ToString::to_string);
            }
            if item.facet.is_none() {
                item.facet = Some(submission.facet.clone());
            }
        }
    }
}

async fn enrich_queue_item_import_states(app: &AppUseCase, items: &mut [DownloadQueueItem]) {
    let import_sources = items
        .iter()
        .filter(|item| queue_item_import_state_eligible(item))
        .map(|item| {
            DownloadSourceIdentity::new(
                Some(item.client_id.as_str()).filter(|value| !value.trim().is_empty()),
                &item.client_type,
                &item.download_client_item_id,
            )
        })
        .collect::<Vec<_>>();

    let delete_sources = items
        .iter()
        .map(|item| {
            (
                Some(item.client_id.clone()).filter(|value| !value.trim().is_empty()),
                item.client_type.clone(),
                item.download_client_item_id.clone(),
                is_history_download_state(&item.state),
            )
        })
        .collect::<Vec<_>>();

    let records = if import_sources.is_empty() {
        Vec::new()
    } else {
        match app
            .services
            .workflow
            .imports
            .list_imports_for_identities(&import_sources)
            .await
        {
            Ok(records) => records,
            Err(error) => {
                tracing::warn!(error = %error, "failed to batch-load import state for queue items");
                Vec::new()
            }
        }
    };
    let delete_commands = match app
        .services
        .workflow
        .download_queue_commands
        .list_latest_delete_commands_for_sources(&delete_sources)
        .await
    {
        Ok(commands) => commands,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to batch-load delete command state for queue items"
            );
            Vec::new()
        }
    };

    let mut manual_records = HashMap::new();
    let mut fallback_records = HashMap::new();
    let mut delete_records = HashMap::new();
    for record in records {
        let key = DownloadSourceIdentity::new(
            record.source_client_id.as_deref(),
            &record.source_system,
            &record.source_ref,
        );
        if record.import_type == ImportType::ManualImport {
            manual_records.entry(key).or_insert(record);
        } else {
            fallback_records.entry(key).or_insert(record);
        }
    }
    for command in delete_commands {
        let key = (
            command.client_id.clone().unwrap_or_default(),
            command.client_type.clone(),
            command.download_client_item_id.clone(),
            command.is_history,
        );
        delete_records.entry(key).or_insert(command);
    }

    for item in items.iter_mut() {
        let import_key = DownloadSourceIdentity::new(
            Some(item.client_id.as_str()).filter(|value| !value.trim().is_empty()),
            &item.client_type,
            &item.download_client_item_id,
        );
        let delete_key = (
            item.client_id.clone(),
            item.client_type.clone(),
            item.download_client_item_id.clone(),
            is_history_download_state(&item.state),
        );
        let legacy_delete_key = (
            String::new(),
            item.client_type.clone(),
            item.download_client_item_id.clone(),
            is_history_download_state(&item.state),
        );
        if queue_item_import_state_eligible(item) {
            if let Some(record) = manual_records.get(&import_key) {
                apply_manual_import_record_to_queue_item(item, record);
            } else if let Some(record) = fallback_records.get(&import_key) {
                apply_import_record_to_queue_item(item, record);
            }
        }
        if let Some(command) = delete_records
            .get(&delete_key)
            .or_else(|| delete_records.get(&legacy_delete_key))
        {
            apply_delete_command_to_queue_item(item, command);
        }
    }
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

fn config_value_is_empty(value: Option<&serde_json::Value>) -> bool {
    match value {
        None | Some(serde_json::Value::Null) => true,
        Some(serde_json::Value::String(value)) => value.trim().is_empty(),
        _ => false,
    }
}

fn config_value_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.trim().to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
    .filter(|value| !value.is_empty())
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

fn download_queue_projection_key(item: &DownloadQueueItem) -> String {
    if item.client_id.trim().is_empty() {
        return format!("{}::{}", item.client_type, item.download_client_item_id);
    }

    format!("{}::{}", item.client_id, item.download_client_item_id)
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
    if let Some(source_title) = tracked
        .source_title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        item.title_name = source_title.to_string();
    }
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

fn synthetic_terminal_download_queue_item(
    tracked: &TrackedDownloadQueueMetadata,
    primary_client: Option<&DownloadClientConfig>,
) -> Option<DownloadQueueItem> {
    let state = match tracked.state {
        TrackedDownloadState::Imported => DownloadQueueState::Completed,
        TrackedDownloadState::Failed => DownloadQueueState::Failed,
        _ => return None,
    };

    let mut item = tracked.client_item.clone();
    item.state = state;
    item.progress_percent = 100;
    item.remaining_seconds = Some(0);
    item.attention_required = matches!(tracked.state, TrackedDownloadState::Failed);

    if matches!(tracked.state, TrackedDownloadState::Imported) {
        item.import_status = Some(ImportStatus::Completed);
        if item.imported_at.is_none() {
            item.imported_at = item.last_updated_at.clone();
        }
    } else if item.import_status.is_none() {
        item.import_status = Some(ImportStatus::Failed);
    }

    if let Some(primary_client) = primary_client {
        if item.client_id.trim().is_empty() {
            item.client_id = primary_client.id.clone();
        }
        if item.client_name.trim().is_empty() {
            item.client_name = primary_client.name.clone();
        }
        if item.client_type.trim().is_empty() {
            item.client_type = primary_client.client_type.clone();
        }
    }

    Some(item)
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
                    client_id: Some(previous_item.client_id.clone())
                        .filter(|value| !value.trim().is_empty()),
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
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .indexer_configs
            .list(provider_filter.map(|provider| provider.trim().to_lowercase()))
            .await
    }

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

    pub fn queue_managed_indexer_sync(&self, actor: &User, config_id: &str) {
        let config_id = config_id.trim().to_string();
        if config_id.is_empty() {
            return;
        }

        let app = self.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            if let Err(error) = app.sync_indexer_config(&actor, &config_id).await {
                tracing::warn!(
                    config_id = %config_id,
                    error = %error,
                    "background managed indexer sync failed"
                );
            }
        });
    }

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
            self.queue_managed_indexer_sync(actor, &created.id);
        }
        self.publish_indexers_changed();
        Ok(created)
    }

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
                self.queue_managed_indexer_sync(actor, &updated.id);
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

    pub async fn list_download_client_configs(
        &self,
        actor: &User,
        client_type: Option<String>,
    ) -> AppResult<Vec<DownloadClientConfig>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

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

    async fn primary_download_client(&self) -> AppResult<Option<DownloadClientConfig>> {
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
            return Ok(None);
        }

        enabled_clients.sort_by_key(|config| config.client_priority);
        Ok(enabled_clients.into_iter().next())
    }

    async fn enrich_download_queue_items(
        &self,
        primary_client: Option<&DownloadClientConfig>,
        mut items: Vec<DownloadQueueItem>,
        use_tracked_runtime_snapshot: bool,
    ) -> Vec<DownloadQueueItem> {
        enrich_download_queue_items_from_submissions(self, &mut items).await;

        if use_tracked_runtime_snapshot {
            match tokio::time::timeout(
                TRACKED_DOWNLOAD_SNAPSHOT_READ_BUDGET,
                self.runtime.acquisition.tracked_download_snapshot.read(),
            )
            .await
            {
                Ok(snapshot) => {
                    let existing_ids = items
                        .iter()
                        .map(|item| {
                            tracked_download_id(
                                Some(item.client_id.as_str()),
                                &item.client_type,
                                &item.download_client_item_id,
                            )
                        })
                        .collect::<HashSet<_>>();
                    for item in &mut items {
                        let tracked_id = tracked_download_id(
                            Some(item.client_id.as_str()),
                            &item.client_type,
                            &item.download_client_item_id,
                        );
                        if let Some(metadata) = snapshot.get(&tracked_id) {
                            apply_tracked_download_queue_metadata(item, metadata);
                        }
                    }
                    items.extend(snapshot.iter().filter_map(|(tracked_id, metadata)| {
                        if existing_ids.contains(tracked_id) {
                            return None;
                        }
                        synthetic_terminal_download_queue_item(metadata, primary_client).map(
                            |mut item| {
                                if item.download_client_item_id.trim().is_empty() {
                                    item.download_client_item_id = tracked_id.to_string();
                                }
                                apply_tracked_download_queue_metadata(&mut item, metadata);
                                item
                            },
                        )
                    }));
                }
                Err(_) => {
                    tracing::warn!(
                        budget_ms = TRACKED_DOWNLOAD_SNAPSHOT_READ_BUDGET.as_millis() as u64,
                        item_count = items.len(),
                        "download queue enrichment timed out reading tracked snapshot; returning degraded client/persisted state"
                    );
                }
            }
        }

        let mut items = dedupe_download_queue_items(items)
            .into_iter()
            .map(|item| {
                let mut mapped = item;
                if let Some(primary_client) = primary_client {
                    if mapped.client_id.is_empty() {
                        mapped.client_id = primary_client.id.clone();
                    }
                    if mapped.client_name.is_empty() {
                        mapped.client_name = primary_client.name.clone();
                    }
                    if mapped.client_type.is_empty() {
                        mapped.client_type = primary_client.client_type.clone();
                    }
                }
                mapped
            })
            .collect::<Vec<_>>();

        enrich_queue_item_import_states(self, &mut items).await;

        items
            .into_iter()
            .map(|item| annotate_download_queue_item(item, primary_client))
            .collect()
    }

    async fn collect_download_snapshot_items(
        &self,
        include_queue: bool,
        include_recent_history: bool,
        use_tracked_runtime_snapshot: bool,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let primary_client = match self.primary_download_client().await? {
            Some(client) => client,
            None => return Ok(Vec::new()),
        };

        let queue_items = if include_queue {
            self.services
                .integrations
                .download_client
                .list_queue()
                .await?
        } else {
            Vec::new()
        };
        let history_items = if include_recent_history {
            // The queue poller and Activity snapshot only need a recent window of
            // history. Older completed items can still be recovered through the
            // explicit history page or manual import flows without forcing an
            // unbounded history scan every 2 seconds.
            self.services
                .integrations
                .download_client
                .list_recent_activity(DOWNLOAD_QUEUE_RECENT_ACTIVITY_LIMIT)
                .await?
        } else {
            Vec::new()
        };

        let mut items: Vec<DownloadQueueItem> = queue_items;
        items.extend(history_items);
        Ok(self
            .enrich_download_queue_items(Some(&primary_client), items, use_tracked_runtime_snapshot)
            .await)
    }

    async fn collect_download_snapshot_items_for_title(
        &self,
        title_id: &str,
        include_queue: bool,
        include_recent_history: bool,
        use_tracked_runtime_snapshot: bool,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let primary_client = match self.primary_download_client().await? {
            Some(client) => client,
            None => return Ok(Vec::new()),
        };

        let queue_items = if include_queue {
            self.services
                .integrations
                .download_client
                .list_queue_for_title(title_id)
                .await?
        } else {
            Vec::new()
        };
        let history_items = if include_recent_history {
            self.services
                .integrations
                .download_client
                .list_recent_activity_for_title(title_id, DOWNLOAD_QUEUE_RECENT_ACTIVITY_LIMIT)
                .await?
        } else {
            Vec::new()
        };

        let mut items: Vec<DownloadQueueItem> = queue_items;
        items.extend(history_items);

        Ok(self
            .enrich_download_queue_items(Some(&primary_client), items, use_tracked_runtime_snapshot)
            .await
            .into_iter()
            .filter(|item| item.title_id.as_deref() == Some(title_id))
            .collect())
    }

    pub(crate) async fn resolve_manual_import_source(
        &self,
        client_id: Option<&str>,
        client_type: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<ManualImportSourceResolution> {
        let source_ref = download_client_item_id.trim();
        if source_ref.is_empty() {
            return Ok(ManualImportSourceResolution::NotEligible {
                message: "download client item id is required".to_string(),
            });
        }

        let mut items = self
            .services
            .integrations
            .download_client
            .list_queue()
            .await?;
        items.extend(
            self.services
                .integrations
                .download_client
                .list_recent_activity(DOWNLOAD_QUEUE_RECENT_ACTIVITY_LIMIT)
                .await?,
        );
        if let Some(item) = items.iter().find(|item| {
            source_identity_matches(
                &item.client_id,
                &item.client_type,
                &item.download_client_item_id,
                client_id,
                client_type,
                source_ref,
            )
        }) {
            return match item.state {
                DownloadQueueState::Failed => Ok(ManualImportSourceResolution::SourceFailed {
                    message: source_failed_message(item),
                }),
                DownloadQueueState::Completed | DownloadQueueState::ImportPending => {
                    let completed = self
                        .find_completed_manual_import_source(client_id, client_type, source_ref)
                        .await?;
                    Ok(ManualImportSourceResolution::Eligible { completed })
                }
                other => Ok(ManualImportSourceResolution::NotEligible {
                    message: format!(
                        "download source {source_ref} is not ready for import; current state is {other:?}"
                    ),
                }),
            };
        }

        let completed = self
            .find_completed_manual_import_source(client_id, client_type, source_ref)
            .await?;
        if completed.is_some() {
            Ok(ManualImportSourceResolution::Eligible { completed })
        } else {
            Ok(ManualImportSourceResolution::NotEligible {
                message: format!("download source {source_ref} is no longer available for import"),
            })
        }
    }

    async fn find_completed_manual_import_source(
        &self,
        client_id: Option<&str>,
        client_type: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<Option<CompletedDownload>> {
        let completed_downloads = self
            .services
            .integrations
            .download_client
            .list_completed_downloads()
            .await?;
        Ok(completed_downloads.into_iter().find(|download| {
            source_identity_matches(
                &download.client_id,
                &download.client_type,
                &download.download_client_item_id,
                client_id,
                client_type,
                download_client_item_id,
            )
        }))
    }

    async fn collect_download_queue_items(
        &self,
        include_all_activity: bool,
        include_history_only: bool,
        include_import_activity: bool,
        activity_filter: DownloadActivityFilter,
        use_tracked_runtime_snapshot: bool,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if include_history_only {
            let mut items = self
                .collect_download_snapshot_items(false, true, use_tracked_runtime_snapshot)
                .await?
                .into_iter()
                .filter(|item| is_history_download_state(&item.state))
                .collect::<Vec<_>>();
            items.sort_by(|left, right| {
                parse_sort_value(
                    right.last_updated_at.as_deref(),
                    left.last_updated_at.as_deref(),
                )
            });
            items.truncate(50);
            return Ok(items);
        }

        let mut items = self
            .collect_download_snapshot_items(true, false, use_tracked_runtime_snapshot)
            .await?
            .into_iter()
            .filter(|item| include_all_activity || item.is_scryer_origin)
            .filter(|item| {
                matches_download_queue_filter(item, false, include_import_activity, activity_filter)
            })
            .collect::<Vec<_>>();
        sort_download_queue_items(&mut items);
        Ok(items)
    }

    async fn collect_download_queue_items_for_title(
        &self,
        title_id: &str,
        include_all_activity: bool,
        include_history_only: bool,
        include_import_activity: bool,
        activity_filter: DownloadActivityFilter,
        use_tracked_runtime_snapshot: bool,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if include_history_only {
            let mut items = self
                .collect_download_snapshot_items_for_title(
                    title_id,
                    false,
                    true,
                    use_tracked_runtime_snapshot,
                )
                .await?
                .into_iter()
                .filter(|item| is_history_download_state(&item.state))
                .collect::<Vec<_>>();
            items.sort_by(|left, right| {
                parse_sort_value(
                    right.last_updated_at.as_deref(),
                    left.last_updated_at.as_deref(),
                )
            });
            items.truncate(50);
            return Ok(items);
        }

        let mut items = self
            .collect_download_snapshot_items_for_title(
                title_id,
                true,
                false,
                use_tracked_runtime_snapshot,
            )
            .await?
            .into_iter()
            .filter(|item| include_all_activity || item.is_scryer_origin)
            .filter(|item| {
                matches_download_queue_filter(item, false, include_import_activity, activity_filter)
            })
            .collect::<Vec<_>>();
        sort_download_queue_items(&mut items);
        Ok(items)
    }

    pub async fn list_download_queue(
        &self,
        actor: &User,
        include_all_activity: bool,
        include_history_only: bool,
        include_import_activity: bool,
        activity_filter: DownloadActivityFilter,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.require_any_library_permission(actor, scryer_domain::LibraryPermission::View)
            .await?;
        let items = self
            .collect_download_queue_items(
                include_all_activity,
                include_history_only,
                include_import_activity,
                activity_filter,
                true,
            )
            .await?;
        self.filter_download_queue_items_for_permission(
            actor,
            items,
            scryer_domain::LibraryPermission::View,
        )
        .await
    }

    async fn require_title_library_permission(
        &self,
        actor: &User,
        title_id: &str,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Title> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(actor, &title.library_id, permission)
            .await?;
        Ok(title)
    }

    async fn require_any_library_permission(
        &self,
        actor: &User,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<()> {
        if self
            .authorized_library_ids(actor, None, permission)
            .await?
            .is_empty()
        {
            Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    async fn filter_download_queue_items_for_permission(
        &self,
        actor: &User,
        items: Vec<DownloadQueueItem>,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, permission)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        if allowed_library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let can_view_operational_history = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let mut title_library_cache = HashMap::<String, Option<String>>::new();
        let mut visible = Vec::new();
        for item in items {
            let allowed = if let Some(title_id) = item.title_id.as_deref() {
                let library_id = if let Some(cached) = title_library_cache.get(title_id) {
                    cached.clone()
                } else {
                    let library_id = self
                        .services
                        .catalog
                        .titles
                        .get_by_id(title_id)
                        .await?
                        .map(|title| title.library_id);
                    title_library_cache.insert(title_id.to_string(), library_id.clone());
                    library_id
                };
                library_id
                    .as_ref()
                    .map(|library_id| allowed_library_ids.contains(library_id))
                    .unwrap_or(can_view_operational_history)
            } else {
                can_view_operational_history
            };
            if allowed {
                visible.push(item);
            }
        }
        Ok(visible)
    }

    async fn require_download_item_permission(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: Option<&str>,
        download_client_item_id: &str,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<()> {
        let item = self
            .find_download_queue_item_raw(client_id, client_type, download_client_item_id)
            .await?;
        if let Some(item) = item
            && let Some(title_id) = item.title_id.as_deref()
        {
            self.require_title_library_permission(actor, title_id, permission)
                .await?;
            return Ok(());
        }
        self.require_any_library_permission(actor, permission).await
    }

    async fn require_completed_download_permission(
        &self,
        actor: &User,
        completed: &CompletedDownload,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<()> {
        if let Some(title_id) =
            crate::import_parameters::extract_parameter(&completed.parameters, "*scryer_title_id")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        {
            self.require_title_library_permission(actor, &title_id, permission)
                .await?;
            Ok(())
        } else {
            self.require_any_library_permission(actor, permission).await
        }
    }

    async fn find_download_queue_item_raw(
        &self,
        client_id: Option<&str>,
        client_type: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<Option<DownloadQueueItem>> {
        let target_download_client_item_id = download_client_item_id.trim();
        if target_download_client_item_id.is_empty() {
            return Err(AppError::Validation(
                "download client item id is required".to_string(),
            ));
        }

        let normalized_client_type = client_type
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());
        let normalized_client_id = client_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let items = self
            .collect_download_snapshot_items(true, true, true)
            .await?;
        Ok(items.into_iter().find(|item| {
            item.download_client_item_id == target_download_client_item_id
                && normalized_client_id
                    .as_ref()
                    .is_none_or(|client_id| item.client_id == *client_id)
                && normalized_client_type
                    .as_ref()
                    .is_none_or(|client_type| item.client_type.eq_ignore_ascii_case(client_type))
        }))
    }

    async fn collect_download_history_items_for_actor(
        &self,
        actor: &User,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let items = self.collect_download_history_items(true).await?;
        self.filter_download_queue_items_for_permission(actor, items, permission)
            .await
    }

    async fn collect_download_snapshot_items_for_actor(
        &self,
        actor: &User,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let items = self
            .collect_download_snapshot_items(true, true, true)
            .await?;
        self.filter_download_queue_items_for_permission(actor, items, permission)
            .await
    }

    pub async fn list_download_queue_for_title(
        &self,
        actor: &User,
        title_id: &str,
        include_all_activity: bool,
        include_history_only: bool,
        include_import_activity: bool,
        activity_filter: DownloadActivityFilter,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.require_title_library_permission(
            actor,
            title_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        self.collect_download_queue_items_for_title(
            title_id,
            include_all_activity,
            include_history_only,
            include_import_activity,
            activity_filter,
            true,
        )
        .await
    }

    pub async fn list_download_import_page(
        &self,
        actor: &User,
        limit: usize,
        offset: usize,
        filter: DownloadImportFilter,
    ) -> AppResult<DownloadImportPage> {
        self.require_any_library_permission(
            actor,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;

        let limit = limit.clamp(1, 100);
        let items = self
            .collect_download_history_items_for_actor(
                actor,
                scryer_domain::LibraryPermission::ResolveImports,
            )
            .await?
            .into_iter()
            .filter(|item| matches_download_import_filter(item, filter))
            .collect::<Vec<_>>();

        let mut items = items;
        items.sort_by(|left, right| {
            let left_rank = match classify_download_queue_item(left).import_filter {
                Some(DownloadImportFilter::Importing) => 0,
                Some(DownloadImportFilter::Pending) => 1,
                Some(DownloadImportFilter::Blocked) => 2,
                Some(DownloadImportFilter::Failed) => 3,
                _ => 4,
            };
            let right_rank = match classify_download_queue_item(right).import_filter {
                Some(DownloadImportFilter::Importing) => 0,
                Some(DownloadImportFilter::Pending) => 1,
                Some(DownloadImportFilter::Blocked) => 2,
                Some(DownloadImportFilter::Failed) => 3,
                _ => 4,
            };
            left_rank.cmp(&right_rank).then_with(|| {
                parse_sort_value(
                    right.last_updated_at.as_deref(),
                    left.last_updated_at.as_deref(),
                )
            })
        });

        let total_count = items.len();
        let page_items = items
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        let has_more = offset.saturating_add(page_items.len()) < total_count;

        Ok(DownloadImportPage {
            items: page_items,
            has_more,
            total_count,
        })
    }

    pub async fn count_download_import_items(
        &self,
        actor: &User,
        filter: DownloadImportFilter,
    ) -> AppResult<i64> {
        self.require_any_library_permission(
            actor,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;

        let count = self
            .collect_download_history_items_for_actor(
                actor,
                scryer_domain::LibraryPermission::ResolveImports,
            )
            .await?
            .into_iter()
            .filter(|item| matches_download_import_filter(item, filter))
            .count();

        Ok(count as i64)
    }

    async fn collect_download_history_items(
        &self,
        use_tracked_runtime_snapshot: bool,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let primary_client = match self.primary_download_client().await? {
            Some(client) => client,
            None => return Ok(Vec::new()),
        };
        let items = self
            .services
            .integrations
            .download_client
            .list_history()
            .await?;

        Ok(self
            .enrich_download_queue_items(Some(&primary_client), items, use_tracked_runtime_snapshot)
            .await)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "download-history queries mirror the user-visible filter surface explicitly"
    )]
    pub async fn list_download_history_page(
        &self,
        actor: &User,
        limit: usize,
        offset: usize,
        filters: Option<Vec<DownloadHistoryFilter>>,
        client_ids: Option<Vec<String>>,
        scryer_submitted_only: bool,
        sort: Option<DownloadHistorySort>,
    ) -> AppResult<DownloadHistoryPage> {
        self.require_any_library_permission(actor, scryer_domain::LibraryPermission::View)
            .await?;

        let limit = limit.clamp(1, 50);
        let normalized_client_ids = client_ids.map(|ids| {
            ids.into_iter()
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect::<HashSet<_>>()
        });
        let mut items = self
            .collect_download_history_items_for_actor(actor, scryer_domain::LibraryPermission::View)
            .await?
            .into_iter()
            .filter(|item| {
                matches!(
                    classify_download_queue_item(item).bucket,
                    DownloadQueueBucket::HistorySuccess | DownloadQueueBucket::HistoryFailed
                )
            })
            .collect::<Vec<_>>();
        items.retain(|item| matches_download_history_filters(item, filters.as_deref()));
        if scryer_submitted_only {
            items.retain(|item| item.is_scryer_origin);
        }
        let available_clients = collect_download_client_filter_options(&items);
        items.retain(|item| {
            matches_download_history_client_ids(item, normalized_client_ids.as_ref())
        });
        if let Some(sort) = sort {
            sort_download_history_items(&mut items, sort);
        } else {
            items.sort_by(|left, right| {
                parse_sort_value(
                    right.last_updated_at.as_deref(),
                    left.last_updated_at.as_deref(),
                )
            });
        }

        let total_count = items.len();
        let page_items = items
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        let has_more = offset.saturating_add(page_items.len()) < total_count;

        Ok(DownloadHistoryPage {
            items: page_items,
            has_more,
            total_count,
            available_clients,
        })
    }

    pub async fn list_download_queue_snapshot(
        &self,
        actor: &User,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.require_any_library_permission(actor, scryer_domain::LibraryPermission::View)
            .await?;
        self.collect_download_snapshot_items_for_actor(
            actor,
            scryer_domain::LibraryPermission::View,
        )
        .await
    }

    pub async fn find_download_queue_item(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<Option<DownloadQueueItem>> {
        self.require_any_library_permission(actor, scryer_domain::LibraryPermission::View)
            .await?;
        let item = self
            .find_download_queue_item_raw(client_id, client_type, download_client_item_id)
            .await?;
        let Some(item) = item else {
            return Ok(None);
        };
        let visible = self
            .filter_download_queue_items_for_permission(
                actor,
                vec![item],
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        Ok(visible.into_iter().next())
    }

    pub async fn find_download_queue_scope(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<SubmissionScope>> {
        self.require_any_library_permission(actor, scryer_domain::LibraryPermission::View)
            .await?;

        let submission = self
            .services
            .workflow
            .download_submissions
            .find_by_client_item_id(&DownloadSourceIdentity::new(
                client_id,
                client_type,
                download_client_item_id,
            ))
            .await?;
        if let Some(submission) = submission.as_ref() {
            let Some(title) = self
                .services
                .catalog
                .titles
                .get_by_id(&submission.title_id)
                .await?
            else {
                tracing::warn!(
                    title_id = %submission.title_id,
                    client_type,
                    download_client_item_id,
                    "download submission scope refers to a missing title; ignoring stale scope"
                );
                return Ok(None);
            };
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        }
        Ok(submission.map(|submission| submission.scope))
    }

    pub fn subscribe_download_queue(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<Vec<DownloadQueueItem>>> {
        if !actor
            .authorization
            .has_any_library_permission(scryer_domain::LibraryPermission::View)
        {
            return Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ));
        }
        let (tx, rx) = broadcast::channel(32);
        let app = self.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            let event_types = vec![
                DomainEventType::DownloadQueueItemUpserted,
                DomainEventType::DownloadQueueItemRemoved,
            ];
            let mut wake_rx = app.runtime.events.domain_event_broadcast.subscribe();
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

            let initial_items = match app.list_download_queue_snapshot(&actor).await {
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

            let initial = match app
                .filter_download_queue_items_for_permission(
                    &actor,
                    sorted_download_queue_items(&items),
                    scryer_domain::LibraryPermission::View,
                )
                .await
            {
                Ok(items) => items,
                Err(error) => {
                    tracing::warn!("download queue subscription initial filter failed: {error}");
                    return;
                }
            };
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
                    if apply_download_queue_projection_event(&mut items, &event).is_some() {
                        let snapshot = match app
                            .filter_download_queue_items_for_permission(
                                &actor,
                                sorted_download_queue_items(&items),
                                scryer_domain::LibraryPermission::View,
                            )
                            .await
                        {
                            Ok(items) => items,
                            Err(error) => {
                                tracing::warn!(
                                    "download queue subscription event filter failed: {error}"
                                );
                                return;
                            }
                        };
                        if tx.send(snapshot).is_err() {
                            return;
                        }
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
        client_id: Option<String>,
        client_type: String,
        download_client_item_id: String,
        files: Option<Vec<crate::ManualImportFileMapping>>,
    ) -> AppResult<String> {
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

        if let Some(title_id) = title_id.as_deref() {
            self.require_title_library_permission(
                actor,
                title_id,
                scryer_domain::LibraryPermission::ResolveImports,
            )
            .await?;
        } else {
            self.require_any_library_permission(
                actor,
                scryer_domain::LibraryPermission::ResolveImports,
            )
            .await?;
        }

        match self
            .resolve_manual_import_source(
                client_id.as_deref(),
                Some(normalized_client_type.as_str()),
                &source_ref,
            )
            .await?
        {
            ManualImportSourceResolution::Eligible { .. } => {}
            ManualImportSourceResolution::SourceFailed { message } => {
                return Err(AppError::Validation(format!(
                    "source_job_failed: {message}"
                )));
            }
            ManualImportSourceResolution::NotEligible { message } => {
                return Err(AppError::Validation(message));
            }
        }

        if let Some(existing) = crate::import_workflow::find_active_manual_import_for_source(
            self,
            client_id.as_deref(),
            normalized_client_type.as_str(),
            &source_ref,
        )
        .await?
        {
            return Ok(existing.id);
        }

        let source_identity = DownloadSourceIdentity::new(
            client_id.as_deref(),
            normalized_client_type.as_str(),
            source_ref.as_str(),
        );

        let payload_json = serde_json::to_string(&crate::ManualImportRequestPayload {
            requested_by_user_id: Some(actor.id.clone()),
            title_id: title_id.clone(),
            download_client_item_id: source_ref.clone(),
            client_id: client_id.clone(),
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
                source_identity,
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
        self.require_completed_download_permission(
            actor,
            &completed,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;

        crate::import_workflow::import_completed_download(self, actor, &completed).await
    }

    pub async fn ignore_tracked_download(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            Some(client_type),
            download_client_item_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        let handle = self
            .runtime
            .acquisition
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .ignore(crate::tracked_downloads::tracked_download_id(
                client_id,
                client_type,
                download_client_item_id,
            ))
            .await?;
        Ok(())
    }

    pub async fn mark_tracked_download_failed(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            Some(client_type),
            download_client_item_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        let handle = self
            .runtime
            .acquisition
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .mark_failed(crate::tracked_downloads::tracked_download_id(
                client_id,
                client_type,
                download_client_item_id,
            ))
            .await?;
        Ok(())
    }

    pub async fn retry_tracked_download_import(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            Some(client_type),
            download_client_item_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        let handle = self
            .runtime
            .acquisition
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .retry_import(crate::tracked_downloads::tracked_download_id(
                client_id,
                client_type,
                download_client_item_id,
            ))
            .await?;
        Ok(())
    }

    pub async fn assign_tracked_download_title(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        title_id: &str,
        scope: SubmissionScope,
    ) -> AppResult<()> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ResolveImports,
        )
        .await?;
        self.services
            .workflow
            .download_submissions
            .record_submission(DownloadSubmission {
                title_id: title.id.clone(),
                facet: title.facet.as_str().to_string(),
                download_client_id: client_id.map(str::to_string),
                download_client_type: client_type.to_string(),
                download_client_item_id: download_client_item_id.to_string(),
                source_hint: None,
                source_kind: None,
                source_title: Some(title.name.clone()),
                request_signature: None,
                scope,
            })
            .await?;
        let handle = self
            .runtime
            .acquisition
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        handle
            .assign_title(
                crate::tracked_downloads::tracked_download_id(
                    client_id,
                    client_type,
                    download_client_item_id,
                ),
                title.id,
            )
            .await?;
        Ok(())
    }

    pub async fn pause_download_queue_item(
        &self,
        actor: &User,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            None,
            download_client_item_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        if let Some(client_id) = client_id.filter(|value| !value.trim().is_empty()) {
            self.services
                .integrations
                .download_client
                .pause_queue_item_for_client(client_id, download_client_item_id)
                .await?;
        } else {
            self.services
                .integrations
                .download_client
                .pause_queue_item(download_client_item_id)
                .await?;
        }
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
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            None,
            download_client_item_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        if let Some(client_id) = client_id.filter(|value| !value.trim().is_empty()) {
            self.services
                .integrations
                .download_client
                .resume_queue_item_for_client(client_id, download_client_item_id)
                .await?;
        } else {
            self.services
                .integrations
                .download_client
                .resume_queue_item(download_client_item_id)
                .await?;
        }
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
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
    ) -> AppResult<crate::DownloadQueueCommandRecord> {
        self.require_download_item_permission(
            actor,
            client_id,
            Some(client_type),
            download_client_item_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let client_type = self.normalize_download_client_type(client_type)?;
        let command = self
            .services
            .workflow
            .download_queue_commands
            .queue_delete_command(
                client_id,
                &client_type,
                download_client_item_id,
                is_history,
                Some(actor.id.as_str()),
            )
            .await?;
        self.emit_download_queue_item_command_issued_event(
            Some(actor.id.clone()),
            download_client_item_id.to_string(),
            scryer_domain::DownloadQueueCommandAction::Delete,
        )
        .await;
        Ok(command)
    }

    pub async fn get_download_client_config(
        &self,
        actor: &User,
        client_id: &str,
    ) -> AppResult<Option<DownloadClientConfig>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
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
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::Validation(
                "download client name is required".into(),
            ));
        }

        let client_type = self.normalize_download_client_type(input.client_type)?;
        let config_json = self.normalize_download_client_config_json(input.config_json)?;
        crate::parse_download_client_remote_path_mappings(&config_json)?;

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
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
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
            Some(value) => {
                let normalized = self.normalize_download_client_config_json(value)?;
                crate::parse_download_client_remote_path_mappings(&normalized)?;
                Some(normalized)
            }
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
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
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
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .download_client_configs
            .reorder(ordered_ids)
            .await
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

pub async fn start_download_queue_poller(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
    mut command_rx: tokio::sync::mpsc::Receiver<crate::tracked_downloads::TrackedDownloadCommand>,
) {
    use crate::tracked_downloads::{
        TrackedDownloadService, publish_runtime_tracked_download_snapshot_cache,
        tracked_download_id,
    };
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
    let (tracked_work_result_tx, mut tracked_work_result_rx) =
        tokio::sync::mpsc::unbounded_channel::<TrackedDownloadBackgroundWorkResult>();
    let mut tracked_work_in_flight = HashSet::new();

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
                        handle_tracked_download_command(
                            &app,
                            &actor,
                            &mut tracker,
                            &mut tracked_work_in_flight,
                            &tracked_work_result_tx,
                            command,
                        )
                        .await;
                    }
                    None => {
                        commands_open = false;
                    }
                }
            }
            maybe_result = tracked_work_result_rx.recv(), if !tracked_work_in_flight.is_empty() => {
                if let Some(result) = maybe_result {
                    handle_tracked_download_background_work_result(
                        &app,
                        &mut tracker,
                        &mut tracked_work_in_flight,
                        result,
                    )
                    .await;
                }
            }
            _ = interval.tick() => {
                let cycle_started_at = Instant::now();
                match app.collect_download_snapshot_items(true, true, false).await {
                    Ok(mut items) => {
                        let mut seen_ids = HashSet::new();
                        let completed_download_lookup =
                            crate::completed_download_handler::load_completed_download_lookup_for_items(
                                &app,
                                &items,
                                DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT,
                            )
                            .await;

                        // Phase 1: Refresh — track each item and run checks.
                        for item in items.iter() {
                            let id = tracked_download_id(
                                Some(item.client_id.as_str()),
                                &item.client_type,
                                &item.download_client_item_id,
                            );
                            seen_ids.insert(id.clone());

                            let is_new = tracker.find(&id).is_none();
                            tracker.track(&app, item.clone()).await;

                            if let Some(td) = tracker.find(&id)
                                && is_new
                            {
                                if td.state.is_terminal()
                                    || is_history_download_state(&td.client_item.state)
                                {
                                    tracing::debug!(
                                        id = %td.id,
                                        state = ?td.state,
                                        client_state = ?td.client_item.state,
                                        match_type = ?td.match_type,
                                        title_id = ?td.title_id,
                                        client_title_name = %td.client_item.title_name,
                                        "tracked: new background download"
                                    )
                                } else {
                                    tracing::info!(
                                        id = %td.id,
                                        state = ?td.state,
                                        client_state = ?td.client_item.state,
                                        match_type = ?td.match_type,
                                        title_id = ?td.title_id,
                                        client_title_name = %td.client_item.title_name,
                                        "tracked: new download"
                                    )
                                }
                            }

                            if let Some(td) = tracker.find_mut(&id)
                                && matches!(
                                    td.state,
                                    TrackedDownloadState::Downloading
                                        | TrackedDownloadState::ImportPending
                                        | TrackedDownloadState::ImportBlocked
                                )
                            {
                                let state_before = td.state;
                                crate::failed_download_handler::check(td);
                                if td.state != TrackedDownloadState::FailedPending {
                                    crate::completed_download_handler::check_with_lookup(
                                        &app,
                                        td,
                                        completed_download_lookup.as_ref(),
                                    )
                                    .await;
                                }
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
                        reconcile_terminal_tracked_downloads(&app, &mut tracker).await;
                        publish_runtime_tracked_download_snapshot_cache(&app, &tracker).await;

                        // Phase 2: Dispatch — import pending and failed items.
                        let trackable_ids = tracker.get_trackable_ids();
                        let mut published_after_dispatch = false;

                        for id in &trackable_ids {
                            if tracked_work_in_flight.len() >= TRACKED_DOWNLOAD_BACKGROUND_WORKER_LIMIT {
                                break;
                            }
                            if tracked_work_in_flight.contains(id) {
                                continue;
                            }

                            if try_dispatch_tracked_download_background_work(
                                &app,
                                &actor,
                                &mut tracker,
                                &mut tracked_work_in_flight,
                                &tracked_work_result_tx,
                                id,
                            ) {
                                published_after_dispatch = true;
                            }
                        }

                        if published_after_dispatch {
                            publish_runtime_tracked_download_snapshot_cache(&app, &tracker).await;
                        }

                        // Enrich items with tracked state before broadcasting.
                        for item in &mut items {
                            let id = tracked_download_id(
                                Some(item.client_id.as_str()),
                                &item.client_type,
                                &item.download_client_item_id,
                            );
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

                        tracing::debug!(
                            elapsed_ms = cycle_started_at.elapsed().as_millis() as u64,
                            item_count = items.len(),
                            tracked_count = tracker.get_all().len(),
                            active_workers = tracked_work_in_flight.len(),
                            "download queue poller cycle completed"
                        );
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
    actor: &User,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &mut HashSet<String>,
    tracked_work_result_tx: &tokio::sync::mpsc::UnboundedSender<
        TrackedDownloadBackgroundWorkResult,
    >,
    command: crate::tracked_downloads::TrackedDownloadCommand,
) {
    use crate::tracked_downloads::TrackedDownloadCommand;
    use scryer_domain::{TrackedDownloadState, TrackedDownloadStatus};

    match command {
        TrackedDownloadCommand::MarkImported { id, reply } => {
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {id} is busy processing"
                ))));
                return;
            }
            let result = if let Some(td) = tracker.find_mut(&id) {
                td.state = TrackedDownloadState::Imported;
                td.status = TrackedDownloadStatus::Ok;
                td.status_messages.clear();
                tracker
                    .persist_terminal_state(app, &id, TrackedDownloadState::Imported)
                    .await;
                finalize_tracked_terminal_state(app, tracker, &id, TrackedDownloadState::Imported)
                    .await;
                Ok(())
            } else {
                Err(AppError::NotFound(format!("tracked download {id}")))
            };
            if result.is_ok() {
                publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::Ignore { id, reply } => {
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {id} is busy processing"
                ))));
                return;
            }
            let result = if let Some(td) = tracker.find_mut(&id) {
                td.state = TrackedDownloadState::Ignored;
                td.status = TrackedDownloadStatus::Ok;
                td.status_messages.clear();
                tracker
                    .persist_terminal_state(app, &id, TrackedDownloadState::Ignored)
                    .await;
                finalize_tracked_terminal_state(app, tracker, &id, TrackedDownloadState::Ignored)
                    .await;
                Ok(())
            } else {
                Err(AppError::NotFound(format!("tracked download {id}")))
            };
            if result.is_ok() {
                publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::MarkFailed { id, reply } => {
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {id} is busy processing"
                ))));
                return;
            }
            let result = if let Some(td) = tracker.find_mut(&id) {
                td.state = TrackedDownloadState::FailedPending;
                td.status = TrackedDownloadStatus::Error;
                td.status_messages.clear();
                let _ = try_dispatch_tracked_download_background_work(
                    app,
                    actor,
                    tracker,
                    tracked_work_in_flight,
                    tracked_work_result_tx,
                    &id,
                );
                Ok(())
            } else {
                Err(AppError::NotFound(format!("tracked download {id}")))
            };
            if result.is_ok() {
                publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::RetryImport { id, reply } => {
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {id} is busy processing"
                ))));
                return;
            }
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
            if result.is_ok() {
                publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            }
            let _ = reply.send(result);
        }
        TrackedDownloadCommand::AssignTitle {
            id,
            title_id,
            reply,
        } => {
            if tracked_work_in_flight.contains(&id) {
                let _ = reply.send(Err(AppError::Validation(format!(
                    "tracked download {id} is busy processing"
                ))));
                return;
            }
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
            if result.is_ok() {
                publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
            }
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

fn prepare_tracked_download_background_work_dispatch(
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    id: &str,
) -> Option<(TrackedDownloadBackgroundWorkKind, TrackedDownload)> {
    let td = tracker.find_mut(id)?;
    match td.state {
        TrackedDownloadState::ImportPending => {
            crate::completed_download_handler::mark_importing(td);
            Some((TrackedDownloadBackgroundWorkKind::Import, td.clone()))
        }
        TrackedDownloadState::FailedPending => {
            Some((TrackedDownloadBackgroundWorkKind::Failed, td.clone()))
        }
        _ => None,
    }
}

fn try_dispatch_tracked_download_background_work(
    app: &AppUseCase,
    actor: &User,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &mut HashSet<String>,
    result_tx: &tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkResult>,
    id: &str,
) -> bool {
    if tracked_work_in_flight.len() >= TRACKED_DOWNLOAD_BACKGROUND_WORKER_LIMIT
        || tracked_work_in_flight.contains(id)
    {
        return false;
    }

    let Some((kind, tracked)) = prepare_tracked_download_background_work_dispatch(tracker, id)
    else {
        return false;
    };

    tracing::info!(
        id = %id,
        work = kind.as_str(),
        active_workers = tracked_work_in_flight.len() + 1,
        worker_limit = TRACKED_DOWNLOAD_BACKGROUND_WORKER_LIMIT,
        "tracked: dispatched background work"
    );
    tracked_work_in_flight.insert(id.to_string());
    dispatch_tracked_download_background_work(
        app.clone(),
        actor.clone(),
        tracked,
        kind,
        result_tx.clone(),
    );
    true
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

fn dispatch_tracked_download_background_work(
    app: AppUseCase,
    actor: User,
    tracked: crate::tracked_downloads::TrackedDownload,
    kind: TrackedDownloadBackgroundWorkKind,
    result_tx: tokio::sync::mpsc::UnboundedSender<TrackedDownloadBackgroundWorkResult>,
) {
    tokio::spawn(async move {
        let started_at = Instant::now();
        let tracked_id = tracked.id.clone();
        let worker = tokio::spawn(async move {
            let mut tracked = tracked;

            match kind {
                TrackedDownloadBackgroundWorkKind::Import => {
                    let _ =
                        crate::completed_download_handler::import(&app, &actor, &mut tracked).await;
                }
                TrackedDownloadBackgroundWorkKind::Failed => {
                    crate::failed_download_handler::process_failed(&app, &mut tracked).await;
                }
            }

            tracked
        });

        let outcome = match worker.await {
            Ok(tracked) => {
                tracing::info!(
                    id = %tracked.id,
                    work = kind.as_str(),
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    final_state = tracked.state.as_str(),
                    "tracked: background work completed"
                );
                Ok(tracked)
            }
            Err(error) => {
                let message = format!(
                    "tracked {} worker exited before completion: {}",
                    kind.as_str(),
                    error
                );
                tracing::error!(
                    id = %tracked_id,
                    work = kind.as_str(),
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    error = %error,
                    "tracked: background work crashed"
                );
                Err(message)
            }
        };
        let elapsed = started_at.elapsed();
        if result_tx
            .send(TrackedDownloadBackgroundWorkResult {
                id: tracked_id,
                kind,
                outcome,
                elapsed,
            })
            .is_err()
        {
            tracing::debug!(
                work = kind.as_str(),
                "tracked background work result dropped after poller shutdown"
            );
        }
    });
}

async fn handle_tracked_download_background_work_result(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    tracked_work_in_flight: &mut HashSet<String>,
    result: TrackedDownloadBackgroundWorkResult,
) {
    tracked_work_in_flight.remove(&result.id);

    let Some(tracked) = tracker.find_mut(&result.id) else {
        tracing::debug!(
            id = %result.id,
            work = result.kind.as_str(),
            elapsed_ms = result.elapsed.as_millis() as u64,
            "tracked background work finished after tracker entry disappeared"
        );
        return;
    };

    let state = match result.outcome {
        Ok(finished) => {
            merge_tracked_download_background_work_state(tracked, finished);
            tracked.state
        }
        Err(message) => {
            tracked.status = TrackedDownloadStatus::Error;
            tracked.status_messages.clear();
            tracked.status_messages.push(message);
            match result.kind {
                TrackedDownloadBackgroundWorkKind::Import => {
                    tracked.import_attempted = true;
                    tracked.state = TrackedDownloadState::ImportBlocked;
                    TrackedDownloadState::ImportBlocked
                }
                TrackedDownloadBackgroundWorkKind::Failed => {
                    tracked.state = TrackedDownloadState::Failed;
                    TrackedDownloadState::Failed
                }
            }
        }
    };

    if state.is_terminal() {
        tracing::info!(
            id = %result.id,
            state = state.as_str(),
            work = result.kind.as_str(),
            "tracked: persisting worker terminal state"
        );
        let persisted = tracker.persist_terminal_state(app, &result.id, state).await;
        if persisted {
            finalize_tracked_terminal_state(app, tracker, &result.id, state).await;
        }
    }

    publish_runtime_tracked_download_snapshot_cache(app, tracker).await;
}

async fn finalize_tracked_terminal_state(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
    id: &str,
    state: TrackedDownloadState,
) {
    let Some(td) = tracker.find(id) else {
        return;
    };

    let cleanup =
        crate::import::import::reconcile_terminal_download_cleanup_for_tracked(app, td, state)
            .await;
    if crate::import::import::terminal_download_cleanup_is_complete(cleanup) {
        tracker.stop_tracking(id);
    }
}

async fn reconcile_terminal_tracked_downloads(
    app: &AppUseCase,
    tracker: &mut crate::tracked_downloads::TrackedDownloadService,
) {
    let terminal_ids: Vec<(String, TrackedDownloadState)> = tracker
        .get_all()
        .into_iter()
        .filter(|tracked| tracked.state.is_terminal())
        .map(|tracked| (tracked.id.clone(), tracked.state))
        .collect();

    for (id, state) in terminal_ids {
        finalize_tracked_terminal_state(app, tracker, &id, state).await;
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

    if !item.client_id.trim().is_empty() {
        return format!("{}:{}", item.client_id, item.download_client_item_id);
    }

    format!("{}:{}", item.client_type, item.download_client_item_id)
}

fn merge_download_queue_item(existing: &mut DownloadQueueItem, incoming: DownloadQueueItem) {
    if existing.title_id.is_none() {
        existing.title_id = incoming.title_id.clone();
    }
    if existing.episode_id.is_none() {
        existing.episode_id = incoming.episode_id.clone();
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

fn is_history_download_state(state: &DownloadQueueState) -> bool {
    matches!(
        state,
        DownloadQueueState::Completed
            | DownloadQueueState::ImportPending
            | DownloadQueueState::Failed
    )
}

#[cfg(test)]
mod tests {
    use super::{
        DownloadQueueBucket, apply_tracked_download_queue_metadata, classify_download_queue_item,
        dedupe_download_queue_items, derive_download_queue_display_state,
        tracked_download_queue_snapshot,
    };
    use crate::DownloadDisplayState;
    use chrono::Utc;
    use scryer_domain::{
        DownloadQueueItem, DownloadQueueState, ImportStatus, TitleMatchType, TrackedDownloadState,
        TrackedDownloadStatus,
    };

    fn item(id: &str, state: DownloadQueueState) -> DownloadQueueItem {
        DownloadQueueItem {
            id: id.to_string(),
            title_id: None,
            episode_id: None,
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
            delete_status: None,
            delete_error_message: None,
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
    fn dedupe_download_queue_items_keeps_same_native_id_from_different_clients() {
        let mut first = item("job-1", DownloadQueueState::Queued);
        first.client_id = "client-1".to_string();
        let mut second = item("job-1", DownloadQueueState::Queued);
        second.client_id = "client-2".to_string();

        let deduped = dedupe_download_queue_items(vec![first, second]);

        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].client_id, "client-1");
        assert_eq!(deduped[1].client_id, "client-2");
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

    #[test]
    fn failed_source_state_stays_out_of_import_bucket() {
        let mut queue_item = item("job-failed", DownloadQueueState::Failed);
        queue_item.import_status = Some(ImportStatus::Failed);
        queue_item.tracked_state = Some(TrackedDownloadState::ImportBlocked);
        queue_item.import_error_message = Some("manual import failed".to_string());

        let classified = classify_download_queue_item(&queue_item);

        assert_eq!(
            derive_download_queue_display_state(&queue_item),
            DownloadDisplayState::Failed
        );
        assert_eq!(classified.bucket, DownloadQueueBucket::HistoryFailed);
    }

    #[test]
    fn apply_tracked_download_queue_metadata_prefers_source_release_title() {
        let mut queue_item = item("job-1", DownloadQueueState::Downloading);
        queue_item.title_name = "Titanic".to_string();
        let tracked = crate::tracked_downloads::TrackedDownload {
            id: "nzbget:job-1".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item: queue_item.clone(),
            state: TrackedDownloadState::Downloading,
            status: TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("movie".to_string()),
            source_title: Some("Titanic.1997.2160p.UHD.BluRay.x265-GRP".to_string()),
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
            is_trackable: true,
            import_attempted: false,
            path_missing_since: None,
        };
        let metadata = tracked_download_queue_snapshot(&tracked);

        apply_tracked_download_queue_metadata(&mut queue_item, &metadata);

        assert_eq!(
            queue_item.title_name,
            "Titanic.1997.2160p.UHD.BluRay.x265-GRP"
        );
    }
}
