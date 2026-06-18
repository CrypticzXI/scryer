use std::{
    fs::File,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, DownloadClient, DownloadClientAddRequest,
    DownloadClientMarkImportedRequest, DownloadClientStatus, DownloadGrabResult,
    DownloadSourceKind, StagedNzbRef,
};
use scryer_domain::{CompletedDownload, DownloadQueueItem, DownloadQueueState};
use scryer_plugin_sdk::torrent::normalize_info_hash_pair;
use tracing::debug;

use crate::types::{
    DownloadControlAction, DownloadInputKind, DownloadIsolationMode, DownloadItemState,
    EXPORT_DOWNLOAD_ADD, EXPORT_DOWNLOAD_CONTROL, EXPORT_DOWNLOAD_LIST_COMPLETED,
    EXPORT_DOWNLOAD_LIST_HISTORY, EXPORT_DOWNLOAD_LIST_QUEUE,
    EXPORT_DOWNLOAD_LIST_RECENT_COMPLETED, EXPORT_DOWNLOAD_MARK_IMPORTED, EXPORT_DOWNLOAD_STATUS,
    PluginCompletedDownload, PluginDescriptor, PluginDownloadClientAddRequest,
    PluginDownloadClientAddResponse, PluginDownloadClientControlRequest,
    PluginDownloadClientMarkImportedRequest, PluginDownloadClientStatus, PluginDownloadIsolation,
    PluginDownloadItem, PluginDownloadListRecentCompletedRequest, PluginDownloadRelease,
    PluginDownloadRouting, PluginDownloadSource, PluginDownloadTitle, PluginTorrentOptions,
    PluginTorrentQueuePlacement, decode_plugin_result,
};

pub struct WasmDownloadClient {
    plugin: Arc<Mutex<extism::Plugin>>,
    descriptor: PluginDescriptor,
    client_name: String,
    client_id: String,
    http: reqwest::Client,
    no_redirect_http: reqwest::Client,
}

impl WasmDownloadClient {
    pub fn new(
        plugin: extism::Plugin,
        descriptor: PluginDescriptor,
        client_id: String,
        client_name: String,
    ) -> Self {
        Self {
            plugin: Arc::new(Mutex::new(plugin)),
            descriptor,
            client_name,
            client_id,
            http: scryer_outbound_http::plugin_reqwest_client(),
            no_redirect_http: scryer_outbound_http::no_redirect_reqwest_client(),
        }
    }
}

fn parse_timestamp(raw: Option<String>) -> Option<DateTime<Utc>> {
    raw.and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&Utc))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResolvedTorrentSource {
    download_url: Option<String>,
    magnet_uri: Option<String>,
    torrent_bytes_base64: Option<String>,
    torrent_url: Option<String>,
    torrent_file_name: Option<String>,
    torrent_content_type: Option<String>,
    nzb_bytes_base64: Option<String>,
    nzb_file_name: Option<String>,
    nzb_content_type: Option<String>,
}

fn map_source_kind(kind: DownloadSourceKind) -> DownloadInputKind {
    match kind {
        DownloadSourceKind::NzbFile => DownloadInputKind::Nzb,
        DownloadSourceKind::NzbUrl => DownloadInputKind::NzbUrl,
        DownloadSourceKind::TorrentFile => DownloadInputKind::TorrentFile,
        DownloadSourceKind::MagnetUri => DownloadInputKind::MagnetUri,
    }
}

fn map_state(state: DownloadItemState) -> DownloadQueueState {
    match state {
        DownloadItemState::Queued => DownloadQueueState::Queued,
        DownloadItemState::Downloading => DownloadQueueState::Downloading,
        DownloadItemState::Verifying => DownloadQueueState::Verifying,
        DownloadItemState::Repairing => DownloadQueueState::Repairing,
        DownloadItemState::Extracting => DownloadQueueState::Extracting,
        DownloadItemState::Paused => DownloadQueueState::Paused,
        DownloadItemState::Completed | DownloadItemState::Seeding => DownloadQueueState::Completed,
        DownloadItemState::ImportPending => DownloadQueueState::ImportPending,
        DownloadItemState::Failed | DownloadItemState::Error | DownloadItemState::Warning => {
            DownloadQueueState::Failed
        }
    }
}

fn attention_required(item: &PluginDownloadItem) -> bool {
    matches!(
        item.state,
        DownloadItemState::Failed | DownloadItemState::Error | DownloadItemState::Warning
    )
}

fn normalized_plugin_info_hash(raw: Option<&str>) -> Option<String> {
    scryer_application::normalize_torrent_info_hash(raw)
}

fn map_add_response_to_grab_result(
    response: PluginDownloadClientAddResponse,
    request: &DownloadClientAddRequest,
    client_type: &str,
) -> DownloadGrabResult {
    let client_item_id = response.client_item_id;
    let info_hash = normalized_plugin_info_hash(response.info_hash.as_deref()).or_else(|| {
        if matches!(
            request.source_kind,
            Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri)
        ) {
            normalized_plugin_info_hash(Some(client_item_id.as_str()))
        } else {
            None
        }
    });

    DownloadGrabResult {
        job_id: client_item_id,
        client_id: None,
        client_type: client_type.to_string(),
        info_hash,
    }
}

fn map_queue_item(
    item: PluginDownloadItem,
    client_id: &str,
    client_name: &str,
    client_type: &str,
) -> DownloadQueueItem {
    let attention = attention_required(&item);
    let attention_reason = item.message.clone();
    let info_hash = normalized_plugin_info_hash(item.info_hash.as_deref())
        .or_else(|| {
            item.torrent
                .as_ref()
                .and_then(|torrent| normalized_plugin_info_hash(torrent.info_hash_v1.as_deref()))
        })
        .or_else(|| normalized_plugin_info_hash(Some(item.client_item_id.as_str())));
    let observed_identity = scryer_application::observed_download_identity(
        scryer_application::ObservedDownloadIdentityInput {
            download_id: item.download_id.as_deref(),
            parameters: &[],
            info_hash_hint: info_hash.as_deref(),
        },
    );
    DownloadQueueItem {
        id: format!(
            "{client_type}:{}",
            info_hash
                .clone()
                .unwrap_or_else(|| item.client_item_id.clone())
        ),
        title_id: None,
        episode_id: None,
        title_name: item.title,
        facet: None,
        category: item
            .category
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        client_id: client_id.to_string(),
        client_name: client_name.to_string(),
        client_type: client_type.to_string(),
        state: map_state(item.state),
        progress_percent: item.progress_percent.unwrap_or(0),
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        size_bytes: item.total_size_bytes,
        remaining_seconds: item.eta_seconds,
        queued_at: None,
        last_updated_at: None,
        attention_required: attention,
        attention_reason,
        download_client_item_id: item.client_item_id,
        download_id: observed_identity.download_id,
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        is_scryer_origin: false,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }
}

fn map_completed_download(
    item: PluginCompletedDownload,
    client_id: &str,
    client_type: &str,
) -> CompletedDownload {
    let info_hash = normalized_plugin_info_hash(item.info_hash.as_deref())
        .or_else(|| normalized_plugin_info_hash(Some(item.client_item_id.as_str())));
    let observed_identity = scryer_application::observed_download_identity(
        scryer_application::ObservedDownloadIdentityInput {
            download_id: item.download_id.as_deref(),
            parameters: &item.parameters,
            info_hash_hint: info_hash.as_deref(),
        },
    );
    CompletedDownload {
        client_type: client_type.to_string(),
        client_id: client_id.to_string(),
        download_client_item_id: item.client_item_id,
        download_id: observed_identity.download_id,
        name: item.name,
        dest_dir: item.dest_dir,
        category: item.category,
        size_bytes: item.size_bytes,
        completed_at: parse_timestamp(item.completed_at),
        parameters: item.parameters,
    }
}

fn map_history_item_from_completed(
    item: PluginCompletedDownload,
    client_id: &str,
    client_name: &str,
    client_type: &str,
) -> DownloadQueueItem {
    let info_hash = normalized_plugin_info_hash(item.info_hash.as_deref())
        .or_else(|| normalized_plugin_info_hash(Some(item.client_item_id.as_str())));
    let observed_identity = scryer_application::observed_download_identity(
        scryer_application::ObservedDownloadIdentityInput {
            download_id: item.download_id.as_deref(),
            parameters: &item.parameters,
            info_hash_hint: info_hash.as_deref(),
        },
    );
    let download_client_item_id = info_hash.unwrap_or_else(|| item.client_item_id.clone());
    DownloadQueueItem {
        id: format!("{client_type}:{download_client_item_id}"),
        title_id: None,
        episode_id: None,
        title_name: item.name,
        facet: None,
        category: item
            .category
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        client_id: client_id.to_string(),
        client_name: client_name.to_string(),
        client_type: client_type.to_string(),
        state: DownloadQueueState::Completed,
        progress_percent: 100,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        size_bytes: item.size_bytes,
        remaining_seconds: Some(0),
        queued_at: None,
        last_updated_at: item.completed_at,
        attention_required: false,
        attention_reason: None,
        download_client_item_id,
        download_id: observed_identity.download_id,
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        is_scryer_origin: false,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }
}

fn plugin_call_error(operation: &str, error: extism::Error) -> AppError {
    let root_cause = error.root_cause().to_string();
    let detail = if root_cause.trim().is_empty() || root_cause == error.to_string() {
        error.to_string()
    } else {
        root_cause
    };

    AppError::Repository(format!("plugin {operation} failed: {detail}"))
}

fn build_isolation_entries(value: Option<&str>) -> Vec<PluginDownloadIsolation> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };

    [
        DownloadIsolationMode::Category,
        DownloadIsolationMode::Tag,
        DownloadIsolationMode::Label,
        DownloadIsolationMode::View,
    ]
    .into_iter()
    .map(|mode| PluginDownloadIsolation {
        mode,
        value: value.to_string(),
    })
    .collect()
}

fn queue_placement(queue_priority: Option<&str>) -> Option<PluginTorrentQueuePlacement> {
    match queue_priority
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("first") | Some("top") | Some("high") => Some(PluginTorrentQueuePlacement::First),
        Some("last") | Some("bottom") | Some("low") => Some(PluginTorrentQueuePlacement::Last),
        _ => None,
    }
}

fn derive_torrent_file_name(request: &DownloadClientAddRequest) -> Option<String> {
    request
        .source_title
        .clone()
        .or_else(|| request.release_title.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn derive_nzb_file_name(request: &DownloadClientAddRequest) -> Option<String> {
    derive_torrent_file_name(request).map(|value| {
        if value.to_ascii_lowercase().ends_with(".nzb") {
            value
        } else {
            format!("{value}.nzb")
        }
    })
}

fn load_staged_nzb_payload(staged_nzb: &StagedNzbRef) -> AppResult<String> {
    let file = File::open(&staged_nzb.compressed_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to open staged nzb artifact {}: {error}",
            staged_nzb.compressed_path.display()
        ))
    })?;
    let bytes = zstd::stream::decode_all(file).map_err(|error| {
        AppError::Repository(format!(
            "failed to decompress staged nzb artifact {}: {error}",
            staged_nzb.compressed_path.display()
        ))
    })?;
    if staged_nzb.raw_size_bytes > 0 && bytes.len() as u64 != staged_nzb.raw_size_bytes {
        return Err(AppError::Repository(format!(
            "staged nzb artifact {} decompressed to {} bytes, expected {}",
            staged_nzb.id,
            bytes.len(),
            staged_nzb.raw_size_bytes
        )));
    }
    Ok(BASE64.encode(bytes))
}

fn select_plugin_input_kind(
    source_kind: DownloadSourceKind,
    resolved: &ResolvedTorrentSource,
) -> DownloadInputKind {
    if resolved.nzb_bytes_base64.is_some() {
        DownloadInputKind::Nzb
    } else if resolved.magnet_uri.is_some() {
        DownloadInputKind::MagnetUri
    } else if resolved.torrent_bytes_base64.is_some() {
        DownloadInputKind::TorrentBytes
    } else if resolved.torrent_url.is_some() {
        DownloadInputKind::TorrentUrl
    } else {
        map_source_kind(source_kind)
    }
}

fn build_plugin_add_request(
    request: &DownloadClientAddRequest,
    source_kind: DownloadSourceKind,
    resolved: ResolvedTorrentSource,
) -> PluginDownloadClientAddRequest {
    let (info_hash_v1, info_hash_v2) = normalize_info_hash_pair(&PluginDownloadRelease {
        info_hash_hint: request.info_hash_hint.clone(),
        info_hash_v1: request.info_hash_hint.clone(),
        info_hash_v2: request.info_hash_hint.clone(),
        ..PluginDownloadRelease::default()
    });
    let source_preference = [
        (resolved.magnet_uri.is_some(), DownloadInputKind::MagnetUri),
        (
            resolved.torrent_bytes_base64.is_some(),
            DownloadInputKind::TorrentBytes,
        ),
        (
            resolved.torrent_url.is_some(),
            DownloadInputKind::TorrentUrl,
        ),
        (
            matches!(source_kind, DownloadSourceKind::TorrentFile)
                && (resolved.torrent_bytes_base64.is_some()
                    || resolved.torrent_url.is_some()
                    || resolved.download_url.is_some()),
            DownloadInputKind::TorrentFile,
        ),
    ]
    .into_iter()
    .filter_map(|(enabled, kind)| enabled.then_some(kind))
    .collect::<Vec<_>>();
    let isolation = build_isolation_entries(request.category.as_deref());

    PluginDownloadClientAddRequest {
        source: PluginDownloadSource {
            kind: select_plugin_input_kind(source_kind, &resolved),
            download_url: resolved.download_url,
            magnet_uri: resolved.magnet_uri,
            torrent_bytes_base64: resolved.torrent_bytes_base64,
            torrent_url: resolved.torrent_url,
            torrent_file_name: resolved
                .torrent_file_name
                .or_else(|| derive_torrent_file_name(request)),
            torrent_content_type: resolved.torrent_content_type,
            nzb_bytes_base64: resolved.nzb_bytes_base64,
            nzb_file_name: resolved.nzb_file_name,
            nzb_content_type: resolved.nzb_content_type,
            source_title: request.source_title.clone(),
            source_password: request.source_password.clone(),
        },
        release: PluginDownloadRelease {
            download_id: request.download_id.clone(),
            release_title: request
                .release_title
                .clone()
                .or_else(|| request.source_title.clone()),
            import_purpose: Some(request.purpose.as_str().to_string()),
            is_recent: request.is_recent,
            season_pack: request.season_pack,
            indexer_name: request.indexer_name.clone(),
            info_hash_hint: request.info_hash_hint.clone(),
            info_hash_v1,
            info_hash_v2,
            seed_goal_ratio: request.seed_goal_ratio,
            seed_goal_seconds: request.seed_goal_seconds,
        },
        title: PluginDownloadTitle {
            title_id: Some(request.title.id.clone()),
            title_name: request.title.name.clone(),
            media_facet: request.title.facet.as_str().to_string(),
            tags: request.title.tags.clone(),
        },
        routing: PluginDownloadRouting {
            isolation_value: request.category.clone(),
            isolation: isolation.clone(),
            post_import_isolation: isolation,
            queue_priority: request.queue_priority.clone(),
            download_directory: request.download_directory.clone(),
        },
        torrent: matches!(
            source_kind,
            DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri
        )
        .then_some(PluginTorrentOptions {
            source_preference,
            seed_goal_ratio: request.seed_goal_ratio,
            seed_goal_seconds: request.seed_goal_seconds,
            initial_state: None,
            queue_placement: queue_placement(request.queue_priority.as_deref()),
            priority_hint: request.queue_priority.clone(),
            sequential_download: None,
            first_last_piece_priority: None,
            content_layout: None,
            skip_checking: None,
            auto_management: None,
            force_start: None,
            safe_seeding: None,
            anonymity_hops: None,
            selected_file_indices: Vec::new(),
        }),
    }
}

#[async_trait]
impl DownloadClient for WasmDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        let source_hint = request.source_hint.clone();
        let source_kind = request
            .source_kind
            .or_else(|| DownloadSourceKind::infer_from_hint(source_hint.as_deref()))
            .unwrap_or(DownloadSourceKind::TorrentFile);

        // When the source is a .torrent HTTP URL and we have no info_hash_hint,
        // pre-fetch the torrent file so the plugin can compute the hash directly.
        // Some trackers redirect .torrent URLs to magnet URIs — detect that and
        // switch to the magnet path.
        let mut torrent_bytes_base64 = None;
        let mut resolved_magnet_uri: Option<String> = None;
        let mut resolved_download_url = source_hint.clone();
        let mut torrent_url = source_hint.clone().filter(|url| {
            matches!(source_kind, DownloadSourceKind::TorrentFile)
                && (url.starts_with("http://") || url.starts_with("https://"))
        });
        let mut torrent_content_type = None;
        let mut nzb_bytes_base64 = None;
        let mut nzb_file_name = None;
        let mut nzb_content_type = None;
        if matches!(
            source_kind,
            DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl
        ) && let Some(staged_nzb) = request.staged_nzb.as_ref()
        {
            nzb_bytes_base64 = Some(load_staged_nzb_payload(staged_nzb)?);
            nzb_file_name = derive_nzb_file_name(request);
            nzb_content_type = Some("application/x-nzb".to_string());
        }

        if matches!(source_kind, DownloadSourceKind::TorrentFile)
            && request.info_hash_hint.is_none()
            && let Some(url) = source_hint.as_ref()
            && (url.starts_with("http://") || url.starts_with("https://"))
            && !url.starts_with("magnet:")
        {
            match scryer_outbound_http::send_reqwest_request(self.no_redirect_http.get(url)).await {
                Ok(resp) if resp.status().is_redirection() => {
                    if let Some(location) =
                        resp.headers().get("location").and_then(|v| v.to_str().ok())
                    {
                        if location.starts_with("magnet:") {
                            debug!(url = %url, magnet = %location, "torrent URL redirected to magnet");
                            resolved_magnet_uri = Some(location.to_string());
                            resolved_download_url = None;
                            torrent_url = None;
                        } else {
                            resolved_download_url = Some(location.to_string());
                            torrent_url = Some(location.to_string());
                            // Follow the redirect with the normal client
                            if let Ok(resp) =
                                scryer_outbound_http::send_reqwest_request(self.http.get(location))
                                    .await
                                && resp.status().is_success()
                            {
                                let content_type = resp
                                    .headers()
                                    .get(reqwest::header::CONTENT_TYPE)
                                    .and_then(|value| value.to_str().ok())
                                    .map(str::to_string);
                                if let Ok(bytes) = resp.bytes().await
                                    && !bytes.is_empty()
                                {
                                    torrent_content_type = content_type;
                                    debug!(url = %url, bytes = bytes.len(), "pre-fetched torrent file (via redirect)");
                                    torrent_bytes_base64 = Some(BASE64.encode(&bytes));
                                }
                            }
                        }
                    }
                }
                Ok(resp) if resp.status().is_success() => {
                    let response_url = resp.url().to_string();
                    if response_url != *url {
                        resolved_download_url = Some(response_url.clone());
                        torrent_url = Some(response_url);
                    }
                    let content_type = resp
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    match resp.bytes().await {
                        Ok(bytes) if !bytes.is_empty() => {
                            torrent_content_type = content_type;
                            debug!(url = %url, bytes = bytes.len(), "pre-fetched torrent file for hash derivation");
                            torrent_bytes_base64 = Some(BASE64.encode(&bytes));
                        }
                        Ok(_) => {
                            debug!(url = %url, "torrent file fetch returned empty body")
                        }
                        Err(e) => {
                            debug!(url = %url, error = %e, "torrent file body read failed")
                        }
                    }
                }
                Ok(resp) => {
                    debug!(url = %url, status = %resp.status(), "torrent file fetch returned non-success")
                }
                Err(e) => debug!(url = %url, error = %e, "torrent file fetch failed"),
            }
        }

        let magnet_uri = resolved_magnet_uri.or_else(|| {
            source_hint
                .as_ref()
                .filter(|v| v.starts_with("magnet:"))
                .cloned()
        });

        let plugin_request = build_plugin_add_request(
            request,
            source_kind,
            ResolvedTorrentSource {
                download_url: resolved_download_url,
                magnet_uri,
                torrent_bytes_base64,
                torrent_url,
                torrent_file_name: derive_torrent_file_name(request),
                torrent_content_type,
                nzb_bytes_base64,
                nzb_file_name,
                nzb_content_type,
            },
        );

        let input = serde_json::to_string(&plugin_request).map_err(|e| {
            AppError::Repository(format!("failed to serialize plugin request: {e}"))
        })?;

        let plugin = Arc::clone(&self.plugin);
        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<&str, String>(EXPORT_DOWNLOAD_ADD, &input)
                .map_err(|e| plugin_call_error(&format!("{EXPORT_DOWNLOAD_ADD}()"), e))
        })
        .await
        .map_err(|e| AppError::download_submit_unavailable(format!("plugin task panicked: {e}")))?
        .map_err(AppError::into_download_submit_unavailable)?;

        let response: PluginDownloadClientAddResponse =
            decode_plugin_result(&output, EXPORT_DOWNLOAD_ADD)
                .map_err(AppError::into_download_submit_unavailable)?;
        Ok(map_add_response_to_grab_result(
            response,
            request,
            self.descriptor.provider_type(),
        ))
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let plugin = Arc::clone(&self.plugin);
        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<(), String>(EXPORT_DOWNLOAD_LIST_QUEUE, ())
                .map_err(|e| plugin_call_error(&format!("{EXPORT_DOWNLOAD_LIST_QUEUE}()"), e))
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))??;

        let items: Vec<PluginDownloadItem> =
            decode_plugin_result(&output, EXPORT_DOWNLOAD_LIST_QUEUE)?;

        Ok(items
            .into_iter()
            .filter(|item| {
                !matches!(
                    item.state,
                    DownloadItemState::Completed
                        | DownloadItemState::Seeding
                        | DownloadItemState::Failed
                        | DownloadItemState::Error
                )
            })
            .map(|item| {
                map_queue_item(
                    item,
                    &self.client_id,
                    &self.client_name,
                    self.descriptor.provider_type(),
                )
            })
            .collect())
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let plugin = Arc::clone(&self.plugin);
        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<(), String>(EXPORT_DOWNLOAD_LIST_HISTORY, ())
                .map_err(|e| plugin_call_error(&format!("{EXPORT_DOWNLOAD_LIST_HISTORY}()"), e))
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))??;

        match decode_plugin_result::<Vec<PluginDownloadItem>>(&output, EXPORT_DOWNLOAD_LIST_HISTORY)
        {
            Ok(items) => Ok(items
                .into_iter()
                .filter(|item| {
                    matches!(
                        item.state,
                        DownloadItemState::Completed
                            | DownloadItemState::Seeding
                            | DownloadItemState::Failed
                            | DownloadItemState::Error
                    )
                })
                .map(|item| {
                    map_queue_item(
                        item,
                        &self.client_id,
                        &self.client_name,
                        self.descriptor.provider_type(),
                    )
                })
                .collect()),
            Err(primary_error) => {
                let items: Vec<PluginCompletedDownload> =
                    decode_plugin_result(&output, EXPORT_DOWNLOAD_LIST_HISTORY).map_err(
                        |fallback_error| {
                            AppError::Repository(format!(
                                "{primary_error}; legacy completed-download history decode also failed: {fallback_error}"
                            ))
                        },
                    )?;
                debug!(
                    client_id = %self.client_id,
                    client_name = %self.client_name,
                    provider_type = self.descriptor.provider_type(),
                    "download history used legacy completed-download envelope fallback"
                );
                Ok(items
                    .into_iter()
                    .map(|item| {
                        map_history_item_from_completed(
                            item,
                            &self.client_id,
                            &self.client_name,
                            self.descriptor.provider_type(),
                        )
                    })
                    .collect())
            }
        }
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        let plugin = Arc::clone(&self.plugin);
        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<(), String>(EXPORT_DOWNLOAD_LIST_COMPLETED, ())
                .map_err(|e| plugin_call_error(&format!("{EXPORT_DOWNLOAD_LIST_COMPLETED}()"), e))
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))??;

        let items: Vec<PluginCompletedDownload> =
            decode_plugin_result(&output, EXPORT_DOWNLOAD_LIST_COMPLETED)?;

        Ok(items
            .into_iter()
            .map(|item| {
                map_completed_download(item, &self.client_id, self.descriptor.provider_type())
            })
            .collect())
    }

    async fn list_recent_completed_downloads(
        &self,
        limit: usize,
    ) -> AppResult<Vec<CompletedDownload>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let input = serde_json::to_string(&PluginDownloadListRecentCompletedRequest { limit })
            .map_err(|e| {
                AppError::Repository(format!("failed to serialize plugin request: {e}"))
            })?;
        let plugin = Arc::clone(&self.plugin);
        let (output, export_name) = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            if guard.function_exists(EXPORT_DOWNLOAD_LIST_RECENT_COMPLETED) {
                let output = guard
                    .call::<&str, String>(EXPORT_DOWNLOAD_LIST_RECENT_COMPLETED, &input)
                    .map_err(|e| {
                        plugin_call_error(&format!("{EXPORT_DOWNLOAD_LIST_RECENT_COMPLETED}()"), e)
                    })?;
                Ok((output, EXPORT_DOWNLOAD_LIST_RECENT_COMPLETED))
            } else {
                let output = guard
                    .call::<(), String>(EXPORT_DOWNLOAD_LIST_COMPLETED, ())
                    .map_err(|e| {
                        plugin_call_error(&format!("{EXPORT_DOWNLOAD_LIST_COMPLETED}()"), e)
                    })?;
                Ok((output, EXPORT_DOWNLOAD_LIST_COMPLETED))
            }
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))??;

        let mut items: Vec<PluginCompletedDownload> = decode_plugin_result(&output, export_name)?;
        if export_name == EXPORT_DOWNLOAD_LIST_COMPLETED {
            debug!(
                provider = %self.descriptor.id,
                limit,
                "plugin client used full completed-download fallback for recent completed downloads"
            );
            items.sort_by_key(|item| std::cmp::Reverse(item.completed_at.clone()));
        } else {
            debug!(
                provider = %self.descriptor.id,
                limit,
                "plugin client used bounded recent completed-download export"
            );
        }
        items.truncate(limit);

        Ok(items
            .into_iter()
            .map(|item| {
                map_completed_download(item, &self.client_id, self.descriptor.provider_type())
            })
            .collect())
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        let request = PluginDownloadClientControlRequest {
            action: DownloadControlAction::Pause,
            client_item_id: id.to_string(),
            remove_data: false,
            is_history: false,
        };
        let input = serde_json::to_string(&request).map_err(|e| {
            AppError::Repository(format!("failed to serialize control request: {e}"))
        })?;
        let plugin = Arc::clone(&self.plugin);
        tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            let output = guard
                .call::<&str, String>(EXPORT_DOWNLOAD_CONTROL, &input)
                .map_err(|e| plugin_call_error(&format!("{EXPORT_DOWNLOAD_CONTROL}()"), e))?;
            decode_plugin_result::<()>(&output, EXPORT_DOWNLOAD_CONTROL)
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))?
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        let request = PluginDownloadClientControlRequest {
            action: DownloadControlAction::Resume,
            client_item_id: id.to_string(),
            remove_data: false,
            is_history: false,
        };
        let input = serde_json::to_string(&request).map_err(|e| {
            AppError::Repository(format!("failed to serialize control request: {e}"))
        })?;
        let plugin = Arc::clone(&self.plugin);
        tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            let output = guard
                .call::<&str, String>(EXPORT_DOWNLOAD_CONTROL, &input)
                .map_err(|e| plugin_call_error(&format!("{EXPORT_DOWNLOAD_CONTROL}()"), e))?;
            decode_plugin_result::<()>(&output, EXPORT_DOWNLOAD_CONTROL)
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))?
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        let request = PluginDownloadClientControlRequest {
            action: DownloadControlAction::Remove,
            client_item_id: id.to_string(),
            remove_data: false,
            is_history,
        };
        let input = serde_json::to_string(&request).map_err(|e| {
            AppError::Repository(format!("failed to serialize control request: {e}"))
        })?;
        let plugin = Arc::clone(&self.plugin);
        tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            let output = guard
                .call::<&str, String>(EXPORT_DOWNLOAD_CONTROL, &input)
                .map_err(|e| plugin_call_error(&format!("{EXPORT_DOWNLOAD_CONTROL}()"), e))?;
            decode_plugin_result::<()>(&output, EXPORT_DOWNLOAD_CONTROL)
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))?
    }

    async fn mark_imported(&self, request: &DownloadClientMarkImportedRequest) -> AppResult<()> {
        let input = serde_json::to_string(&PluginDownloadClientMarkImportedRequest {
            client_item_id: request.client_item_id.clone(),
            info_hash: request.info_hash.clone(),
            title_id: request.title_id.clone(),
            title_name: request.title_name.clone(),
            category: request.category.clone(),
            post_import_isolation: build_isolation_entries(request.category.as_deref()),
            imported_path: request.imported_path.clone(),
            download_path: request.download_path.clone(),
        })
        .map_err(|e| {
            AppError::Repository(format!("failed to serialize mark_imported request: {e}"))
        })?;
        let plugin = Arc::clone(&self.plugin);
        tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            let output = guard
                .call::<&str, String>(EXPORT_DOWNLOAD_MARK_IMPORTED, &input)
                .map_err(|e| plugin_call_error(&format!("{EXPORT_DOWNLOAD_MARK_IMPORTED}()"), e))?;
            decode_plugin_result::<()>(&output, EXPORT_DOWNLOAD_MARK_IMPORTED)
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))?
    }

    async fn get_client_status(&self) -> AppResult<DownloadClientStatus> {
        let plugin = Arc::clone(&self.plugin);
        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<(), String>(EXPORT_DOWNLOAD_STATUS, ())
                .map_err(|e| plugin_call_error(&format!("{EXPORT_DOWNLOAD_STATUS}()"), e))
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))??;

        let status: PluginDownloadClientStatus =
            decode_plugin_result(&output, EXPORT_DOWNLOAD_STATUS)?;

        Ok(DownloadClientStatus {
            version: status.version,
            is_localhost: status.is_localhost,
            remote_output_roots: status.remote_output_roots,
            removes_completed_downloads: status.removes_completed_downloads,
            sorting_mode: status.sorting_mode,
            warnings: status.warnings,
        })
    }

    async fn test_connection(&self) -> AppResult<String> {
        let plugin = Arc::clone(&self.plugin);
        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<(), String>(crate::types::EXPORT_DOWNLOAD_TEST_CONNECTION, ())
                .map_err(|e| {
                    plugin_call_error(
                        &format!("{}()", crate::types::EXPORT_DOWNLOAD_TEST_CONNECTION),
                        e,
                    )
                })
        })
        .await
        .map_err(|e| AppError::Repository(format!("plugin task panicked: {e}")))??;

        decode_plugin_result(&output, crate::types::EXPORT_DOWNLOAD_TEST_CONNECTION)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use scryer_plugin_sdk::PluginTorrentItem;

    fn sample_request() -> DownloadClientAddRequest {
        DownloadClientAddRequest {
            title: scryer_domain::Title {
                id: "title-1".to_string(),
                name: "Example".to_string(),
                facet: scryer_domain::MediaFacet::Series,
                library_id: scryer_domain::default_library_id_for_facet(
                    &scryer_domain::MediaFacet::Series,
                ),
                monitored: true,
                tags: Vec::new(),
                external_ids: Vec::new(),
                created_by: None,
                created_at: Utc::now(),
                year: None,
                overview: None,
                poster_url: None,
                poster_source_url: None,
                background_url: None,
                background_source_url: None,
                sort_title: None,
                slug: None,
                imdb_id: None,
                runtime_minutes: None,
                genres: Vec::new(),
                content_status: None,
                language: None,
                first_aired: None,
                network: None,
                studio: None,
                country: None,
                aliases: Vec::new(),
                tagged_aliases: Vec::new(),
                metadata_language: None,
                metadata_fetched_at: None,
                min_availability: None,
                digital_release_date: None,
                folder_path: None,
            },
            purpose: scryer_application::DownloadSubmissionPurpose::Standard,
            download_id: None,
            source_hint: Some("https://tracker.example/release.torrent".to_string()),
            staged_nzb: None,
            source_kind: Some(DownloadSourceKind::TorrentFile),
            source_title: Some("Example.Release.torrent".to_string()),
            source_password: None,
            category: Some("scryer-series".to_string()),
            queue_priority: Some("first".to_string()),
            download_directory: Some("/downloads/series".to_string()),
            release_title: Some("Example.Release".to_string()),
            indexer_name: Some("Torrent Indexer".to_string()),
            info_hash_hint: Some("abcdef0123456789abcdef0123456789abcdef01".to_string()),
            seed_goal_ratio: Some(1.5),
            seed_goal_seconds: Some(3661),
            is_recent: Some(true),
            season_pack: Some(false),
        }
    }

    #[test]
    fn add_response_forwards_plugin_info_hash() {
        let request = sample_request();
        let grab = map_add_response_to_grab_result(
            PluginDownloadClientAddResponse {
                client_item_id: "native-item-id".to_string(),
                info_hash: Some("ABCDEF0123456789ABCDEF0123456789ABCDEF01".to_string()),
            },
            &request,
            "qbittorrent",
        );

        assert_eq!(grab.job_id, "native-item-id");
        assert_eq!(
            grab.info_hash.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef01")
        );
    }

    #[test]
    fn add_response_falls_back_to_hash_shaped_torrent_item_id() {
        let request = sample_request();
        let grab = map_add_response_to_grab_result(
            PluginDownloadClientAddResponse {
                client_item_id: "ABCDEF0123456789ABCDEF0123456789ABCDEF01".to_string(),
                info_hash: None,
            },
            &request,
            "qbittorrent",
        );

        assert_eq!(
            grab.info_hash.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef01")
        );
    }

    #[test]
    fn build_plugin_add_request_populates_v11_torrent_fields() {
        let request = sample_request();
        let plugin_request = build_plugin_add_request(
            &request,
            DownloadSourceKind::TorrentFile,
            ResolvedTorrentSource {
                download_url: request.source_hint.clone(),
                magnet_uri: None,
                torrent_bytes_base64: Some("dG9ycmVudA==".to_string()),
                torrent_url: request.source_hint.clone(),
                torrent_file_name: Some("Example.Release.torrent".to_string()),
                torrent_content_type: Some("application/x-bittorrent".to_string()),
                nzb_bytes_base64: None,
                nzb_file_name: None,
                nzb_content_type: None,
            },
        );

        assert_eq!(plugin_request.source.kind, DownloadInputKind::TorrentBytes);
        assert_eq!(
            plugin_request.source.torrent_content_type.as_deref(),
            Some("application/x-bittorrent")
        );
        assert_eq!(
            plugin_request.release.info_hash_v1.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef01")
        );
        assert_eq!(plugin_request.routing.isolation.len(), 4);
        assert_eq!(
            plugin_request
                .torrent
                .as_ref()
                .and_then(|torrent| torrent.queue_placement),
            Some(PluginTorrentQueuePlacement::First)
        );
        assert_eq!(
            plugin_request
                .torrent
                .as_ref()
                .map(|torrent| torrent.source_preference.clone()),
            Some(vec![
                DownloadInputKind::TorrentBytes,
                DownloadInputKind::TorrentUrl,
                DownloadInputKind::TorrentFile,
            ])
        );
    }

    #[test]
    fn build_plugin_add_request_prefers_magnet_after_redirect() {
        let request = sample_request();
        let plugin_request = build_plugin_add_request(
            &request,
            DownloadSourceKind::TorrentFile,
            ResolvedTorrentSource {
                download_url: None,
                magnet_uri: Some(
                    "magnet:?xt=urn:btih:abcdef0123456789abcdef0123456789abcdef01".to_string(),
                ),
                torrent_bytes_base64: None,
                torrent_url: None,
                torrent_file_name: None,
                torrent_content_type: None,
                nzb_bytes_base64: None,
                nzb_file_name: None,
                nzb_content_type: None,
            },
        );

        assert_eq!(plugin_request.source.kind, DownloadInputKind::MagnetUri);
        assert_eq!(
            plugin_request
                .torrent
                .as_ref()
                .map(|torrent| torrent.source_preference.clone()),
            Some(vec![DownloadInputKind::MagnetUri])
        );
    }

    #[test]
    fn build_plugin_add_request_preserves_nzb_url_without_torrent_projection() {
        let mut request = sample_request();
        request.source_hint = Some("https://indexer.example/download/release.nzb".to_string());
        request.source_kind = Some(DownloadSourceKind::NzbUrl);
        request.source_title = Some("Example.Release.nzb".to_string());
        request.info_hash_hint = None;
        let plugin_request = build_plugin_add_request(
            &request,
            DownloadSourceKind::NzbUrl,
            ResolvedTorrentSource {
                download_url: request.source_hint.clone(),
                magnet_uri: None,
                torrent_bytes_base64: None,
                torrent_url: None,
                torrent_file_name: None,
                torrent_content_type: None,
                nzb_bytes_base64: None,
                nzb_file_name: None,
                nzb_content_type: None,
            },
        );

        assert_eq!(plugin_request.source.kind, DownloadInputKind::NzbUrl);
        assert_eq!(
            plugin_request.source.download_url.as_deref(),
            Some("https://indexer.example/download/release.nzb")
        );
        assert_eq!(plugin_request.source.torrent_url, None);
        assert!(plugin_request.torrent.is_none());
    }

    #[test]
    fn build_plugin_add_request_populates_staged_nzb_fields() {
        let mut request = sample_request();
        request.source_hint = Some("https://indexer.example/download/release.nzb".to_string());
        request.source_kind = Some(DownloadSourceKind::NzbUrl);
        request.source_title = Some("Example.Release".to_string());
        request.info_hash_hint = None;
        let plugin_request = build_plugin_add_request(
            &request,
            DownloadSourceKind::NzbUrl,
            ResolvedTorrentSource {
                download_url: request.source_hint.clone(),
                magnet_uri: None,
                torrent_bytes_base64: None,
                torrent_url: None,
                torrent_file_name: None,
                torrent_content_type: None,
                nzb_bytes_base64: Some("bmti".to_string()),
                nzb_file_name: Some("Example.Release.nzb".to_string()),
                nzb_content_type: Some("application/x-nzb".to_string()),
            },
        );

        assert_eq!(plugin_request.source.kind, DownloadInputKind::Nzb);
        assert_eq!(
            plugin_request.source.nzb_bytes_base64.as_deref(),
            Some("bmti")
        );
        assert_eq!(
            plugin_request.source.nzb_file_name.as_deref(),
            Some("Example.Release.nzb")
        );
        assert_eq!(
            plugin_request.source.nzb_content_type.as_deref(),
            Some("application/x-nzb")
        );
        assert_eq!(plugin_request.source.torrent_url, None);
        assert!(plugin_request.torrent.is_none());
    }

    #[test]
    fn mark_imported_post_import_isolation_matches_legacy_value() {
        let entries = build_isolation_entries(Some("series-cat"));
        assert_eq!(entries.len(), 4);
        assert!(
            entries
                .iter()
                .any(|entry| entry.mode == DownloadIsolationMode::Category)
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.mode == DownloadIsolationMode::Label)
        );
    }

    #[test]
    fn completed_history_fallback_maps_to_completed_queue_item() {
        let queue_item = map_history_item_from_completed(
            PluginCompletedDownload {
                client_item_id: "native-1".to_string(),
                download_id: None,
                info_hash: Some("abcdef0123456789abcdef0123456789abcdef01".to_string()),
                name: "Example Release".to_string(),
                dest_dir: "/downloads/series".to_string(),
                category: Some("series".to_string()),
                output_kind: None,
                content_paths: vec!["/downloads/series/Example.Release.mkv".to_string()],
                size_bytes: Some(1234),
                completed_at: Some("2026-05-02T00:00:00Z".to_string()),
                parameters: vec![],
            },
            "client-1",
            "qBittorrent",
            "qbittorrent",
        );

        assert_eq!(
            queue_item.id,
            "qbittorrent:abcdef0123456789abcdef0123456789abcdef01"
        );
        assert_eq!(queue_item.title_name, "Example Release");
        assert_eq!(queue_item.client_name, "qBittorrent");
        assert_eq!(queue_item.state, DownloadQueueState::Completed);
        assert_eq!(queue_item.category.as_deref(), Some("series"));
        assert_eq!(queue_item.progress_percent, 100);
        assert_eq!(queue_item.remaining_seconds, Some(0));
    }

    #[test]
    fn queue_item_falls_back_to_nested_torrent_info_hash_for_qbit_facades() {
        let queue_item = map_queue_item(
            PluginDownloadItem {
                client_item_id: "native-id-1".to_string(),
                download_id: None,
                info_hash: None,
                title: "Decypharr Queue Item".to_string(),
                state: DownloadItemState::Seeding,
                message: None,
                category: Some("series".to_string()),
                remote_output_path: Some("/downloads/series".to_string()),
                torrent: Some(PluginTorrentItem {
                    info_hash_v1: Some("abcdef0123456789abcdef0123456789abcdef01".to_string()),
                    ..PluginTorrentItem::default()
                }),
                total_size_bytes: Some(2048),
                remaining_size_bytes: Some(0),
                eta_seconds: Some(0),
                progress_percent: Some(100),
                can_move_files: Some(true),
                can_remove: Some(true),
                removed: Some(false),
                raw_state: Some("uploading".to_string()),
                completed_at: Some("2026-05-02T00:00:00Z".to_string()),
            },
            "client-1",
            "Decypharr qBit",
            "qbittorrent",
        );

        assert_eq!(
            queue_item.id,
            "qbittorrent:abcdef0123456789abcdef0123456789abcdef01"
        );
        assert_eq!(
            queue_item.download_client_item_id,
            "native-id-1".to_string()
        );
        assert_eq!(queue_item.state, DownloadQueueState::Completed);
        assert_eq!(queue_item.category.as_deref(), Some("series"));
    }

    #[test]
    fn queue_item_prefers_explicit_plugin_download_identity() {
        let queue_item = map_queue_item(
            PluginDownloadItem {
                client_item_id: "native-id-3".to_string(),
                download_id: Some("scryer-download:plugin-1".to_string()),
                info_hash: Some("abcdef0123456789abcdef0123456789abcdef01".to_string()),
                title: "Plugin Queue Item".to_string(),
                state: DownloadItemState::Downloading,
                message: None,
                category: Some("series".to_string()),
                remote_output_path: Some("/downloads/series".to_string()),
                torrent: None,
                total_size_bytes: Some(2048),
                remaining_size_bytes: Some(1024),
                eta_seconds: Some(60),
                progress_percent: Some(50),
                can_move_files: Some(true),
                can_remove: Some(true),
                removed: Some(false),
                raw_state: None,
                completed_at: None,
            },
            "client-1",
            "Plugin Client",
            "plugin-client",
        );

        assert_eq!(
            queue_item.download_id.as_deref(),
            Some("scryer-download:plugin-1")
        );
    }

    #[test]
    fn completed_download_uses_info_hash_identity_without_replacing_live_handle() {
        let info_hash = "fedcba9876543210fedcba9876543210fedcba98";
        let completed = map_completed_download(
            PluginCompletedDownload {
                client_item_id: "native-id-2".to_string(),
                download_id: None,
                info_hash: Some(info_hash.to_string()),
                name: "Decypharr Completed".to_string(),
                dest_dir: "/downloads/movies".to_string(),
                category: Some("movies".to_string()),
                output_kind: None,
                content_paths: vec!["/downloads/movies/Decypharr.Completed.mkv".to_string()],
                size_bytes: Some(4096),
                completed_at: Some("2026-05-03T00:00:00Z".to_string()),
                parameters: vec![("source".to_string(), "decypharr".to_string())],
            },
            "client-1",
            "qbittorrent",
        );

        assert_eq!(completed.download_client_item_id, "native-id-2");
        assert_eq!(completed.download_id.as_deref(), Some(info_hash));
        assert_eq!(completed.dest_dir, "/downloads/movies");
        assert_eq!(
            completed.parameters,
            vec![("source".to_string(), "decypharr".to_string())]
        );
    }
}
