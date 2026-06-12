const DOWNLOAD_QUEUE_RECENT_ACTIVITY_LIMIT: usize = 100;
const DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT: usize = 100;
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
impl AppUseCase {
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
                        .map(tracked_download_id_for_item)
                        .collect::<HashSet<_>>();
                    for item in &mut items {
                        let tracked_id = tracked_download_id_for_item(item);
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
}
impl AppUseCase {
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
