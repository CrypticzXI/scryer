use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, DOWNLOAD_FEEDBACK_TIMEOUT_MESSAGE, DownloadClient,
    DownloadClientAddRequest, DownloadClientConfigRepository, DownloadClientPluginProvider,
    DownloadClientRemotePathMapping, DownloadClientStatus, DownloadGrabResult, DownloadSourceKind,
    SettingsRepository, StagedNzbRef, StagedNzbStore, accepted_inputs_for_client,
    apply_remote_path_mappings_to_completed_download, apply_remote_path_mappings_to_status,
    parse_download_client_remote_path_mappings,
};
use scryer_domain::{DownloadClientConfig, DownloadQueueItem, MediaFacet};
use scryer_outbound_http::{OutboundHttpClient, RateLimitRegistry, generic_reqwest_client};
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tracing::{debug, warn};

use super::nzbget::NzbgetDownloadClient;
use super::sabnzbd::SabnzbdDownloadClient;
use super::weaver::WeaverDownloadClient;
use super::{
    parse_download_client_config_json, read_config_string, request_source_hint_for_nzb,
    resolve_download_client_base_url, stage_nzb_from_url,
};

const DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY: &str = "download_client.routing";
const LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY: &str = "nzbget.client_routing";
const DOWNLOAD_CLIENT_FEEDBACK_TIMEOUT_SECS: u64 = 10;
const DOWNLOAD_CLIENT_FEEDBACK_BACKOFF_INITIAL_SECS: u64 = 15;
const DOWNLOAD_CLIENT_FEEDBACK_BACKOFF_MAX_SECS: u64 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DownloadFeedbackReadKind {
    Queue,
    TitleQueue,
    History,
    RecentActivity,
    TitleRecentActivity,
    RecentCompletedDownloads,
}

#[derive(Clone, Copy, Debug)]
struct FeedbackReadBackoffState {
    consecutive_failures: u32,
    blocked_until: Instant,
}

fn download_client_remote_path_mappings(
    config: &DownloadClientConfig,
) -> Option<Vec<DownloadClientRemotePathMapping>> {
    match parse_download_client_remote_path_mappings(&config.config_json) {
        Ok(mappings) => Some(mappings),
        Err(error) => {
            warn!(
                client_id = %config.id,
                client = %config.name,
                error = %error,
                "failed to parse remote path mappings for download client"
            );
            None
        }
    }
}

#[derive(Clone)]
pub struct PrioritizedDownloadClientRouter {
    download_client_configs: Arc<dyn DownloadClientConfigRepository>,
    settings: Arc<dyn SettingsRepository>,
    fallback_client: Arc<dyn DownloadClient>,
    staged_nzb_store: Arc<dyn StagedNzbStore>,
    staged_nzb_pipeline_limit: Arc<Semaphore>,
    plugin_provider: Option<Arc<dyn DownloadClientPluginProvider>>,
    outbound_http: OutboundHttpClient,
    feedback_read_timeout: Duration,
    feedback_read_backoff:
        Arc<Mutex<HashMap<(String, DownloadFeedbackReadKind), FeedbackReadBackoffState>>>,
}

#[derive(Clone)]
struct FeedbackTimeoutDownloadClient {
    inner: Arc<dyn DownloadClient>,
    read_timeout: Duration,
}

impl FeedbackTimeoutDownloadClient {
    fn new(inner: Arc<dyn DownloadClient>, read_timeout: Duration) -> Self {
        Self {
            inner,
            read_timeout,
        }
    }

    async fn run_feedback_read<T, F>(&self, future: F) -> AppResult<T>
    where
        F: Future<Output = AppResult<T>> + Send,
        T: Send,
    {
        timeout(self.read_timeout, future).await.map_err(|_| {
            AppError::DownloadFeedbackTimeout(DOWNLOAD_FEEDBACK_TIMEOUT_MESSAGE.to_string())
        })?
    }
}

#[async_trait]
impl DownloadClient for FeedbackTimeoutDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        self.inner.submit_download(request).await
    }

    async fn submit_to_download_queue(
        &self,
        title: &scryer_domain::Title,
        source_hint: Option<String>,
        source_kind: Option<DownloadSourceKind>,
        source_title: Option<String>,
        source_password: Option<String>,
        category: Option<String>,
    ) -> AppResult<DownloadGrabResult> {
        self.inner
            .submit_to_download_queue(
                title,
                source_hint,
                source_kind,
                source_title,
                source_password,
                category,
            )
            .await
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_queue()).await
    }

    async fn list_queue_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_queue_for_title(title_id))
            .await
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_history()).await
    }

    async fn list_history_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_history_page(offset, limit))
            .await
    }

    async fn list_recent_activity(&self, limit: usize) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_recent_activity(limit))
            .await
    }

    async fn list_recent_activity_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.run_feedback_read(self.inner.list_recent_activity_for_title(title_id, limit))
            .await
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        self.inner.list_completed_downloads().await
    }

    async fn list_recent_completed_downloads(
        &self,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        self.inner.list_recent_completed_downloads(limit).await
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        self.inner.pause_queue_item(id).await
    }

    async fn pause_queue_item_for_client(&self, client_id: &str, id: &str) -> AppResult<()> {
        self.inner.pause_queue_item_for_client(client_id, id).await
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        self.inner.resume_queue_item(id).await
    }

    async fn resume_queue_item_for_client(&self, client_id: &str, id: &str) -> AppResult<()> {
        self.inner.resume_queue_item_for_client(client_id, id).await
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        self.inner.delete_queue_item(id, is_history).await
    }

    async fn delete_queue_item_for_client_id(
        &self,
        client_id: &str,
        id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        self.inner
            .delete_queue_item_for_client_id(client_id, id, is_history)
            .await
    }

    async fn delete_queue_item_for_client(
        &self,
        client_type: &str,
        id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        self.inner
            .delete_queue_item_for_client(client_type, id, is_history)
            .await
    }

    async fn mark_imported(
        &self,
        request: &scryer_application::DownloadClientMarkImportedRequest,
    ) -> AppResult<()> {
        self.inner.mark_imported(request).await
    }

    async fn get_client_status(&self) -> AppResult<scryer_application::DownloadClientStatus> {
        self.inner.get_client_status().await
    }

    async fn get_client_status_for_client_id(
        &self,
        client_id: &str,
    ) -> AppResult<scryer_application::DownloadClientStatus> {
        self.inner.get_client_status_for_client_id(client_id).await
    }

    async fn test_connection(&self) -> AppResult<String> {
        self.inner.test_connection().await
    }
}

#[derive(Default)]
struct FeedbackReadSummary {
    successful_clients: usize,
    timed_out_clients: usize,
}

impl FeedbackReadSummary {
    fn record_success(&mut self) {
        self.successful_clients += 1;
    }

    fn record_error(&mut self, error: &AppError) {
        if matches!(error, AppError::DownloadFeedbackTimeout(_)) {
            self.timed_out_clients += 1;
        }
    }

    fn finish(self) -> AppResult<()> {
        if self.successful_clients == 0 && self.timed_out_clients > 0 {
            return Err(AppError::DownloadFeedbackTimeout(
                DOWNLOAD_FEEDBACK_TIMEOUT_MESSAGE.to_string(),
            ));
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DownloadClientRoutingScope {
    Library,
    Facet,
}

struct FacetClientSelection {
    clients: Vec<DownloadClientConfig>,
    disabled_scope: Option<DownloadClientRoutingScope>,
}

struct ResolvedDownloadClientRouting {
    scope: DownloadClientRoutingScope,
    routing_object: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct DownloadClientRoutingEntry {
    enabled: bool,
    category: Option<String>,
    recent_queue_priority: Option<String>,
    older_queue_priority: Option<String>,
    remove_completed: bool,
    remove_failed: bool,
}

impl PrioritizedDownloadClientRouter {
    pub fn new(
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        settings: Arc<dyn SettingsRepository>,
        fallback_client: Arc<dyn DownloadClient>,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
        plugin_provider: Option<Arc<dyn DownloadClientPluginProvider>>,
    ) -> Self {
        Self::with_feedback_read_timeout(
            download_client_configs,
            settings,
            fallback_client,
            staged_nzb_store,
            staged_nzb_pipeline_limit,
            plugin_provider,
            Duration::from_secs(DOWNLOAD_CLIENT_FEEDBACK_TIMEOUT_SECS),
        )
    }

    fn with_feedback_read_timeout(
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        settings: Arc<dyn SettingsRepository>,
        fallback_client: Arc<dyn DownloadClient>,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
        plugin_provider: Option<Arc<dyn DownloadClientPluginProvider>>,
        feedback_read_timeout: Duration,
    ) -> Self {
        let http_client = generic_reqwest_client();
        Self {
            download_client_configs,
            settings,
            fallback_client: Self::wrap_feedback_client(fallback_client, feedback_read_timeout),
            staged_nzb_store,
            staged_nzb_pipeline_limit,
            plugin_provider,
            outbound_http: OutboundHttpClient::new(http_client.clone(), RateLimitRegistry::new()),
            feedback_read_timeout,
            feedback_read_backoff: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn wrap_feedback_client(
        client: Arc<dyn DownloadClient>,
        feedback_read_timeout: Duration,
    ) -> Arc<dyn DownloadClient> {
        Arc::new(FeedbackTimeoutDownloadClient::new(
            client,
            feedback_read_timeout,
        ))
    }

    fn feedback_read_kind_label(kind: DownloadFeedbackReadKind) -> &'static str {
        match kind {
            DownloadFeedbackReadKind::Queue => "queue",
            DownloadFeedbackReadKind::TitleQueue => "title_queue",
            DownloadFeedbackReadKind::History => "history",
            DownloadFeedbackReadKind::RecentActivity => "recent_activity",
            DownloadFeedbackReadKind::TitleRecentActivity => "title_recent_activity",
            DownloadFeedbackReadKind::RecentCompletedDownloads => "recent_completed_downloads",
        }
    }

    fn feedback_read_bypasses_backoff(kind: DownloadFeedbackReadKind) -> bool {
        matches!(
            kind,
            DownloadFeedbackReadKind::TitleQueue | DownloadFeedbackReadKind::TitleRecentActivity
        )
    }

    fn feedback_backoff_duration(consecutive_failures: u32) -> Duration {
        let mut seconds = DOWNLOAD_CLIENT_FEEDBACK_BACKOFF_INITIAL_SECS;
        for _ in 1..consecutive_failures {
            seconds = seconds
                .saturating_mul(2)
                .min(DOWNLOAD_CLIENT_FEEDBACK_BACKOFF_MAX_SECS);
        }
        Duration::from_secs(seconds)
    }

    fn feedback_backoff_remaining(
        &self,
        client_id: &str,
        kind: DownloadFeedbackReadKind,
    ) -> Option<Duration> {
        if Self::feedback_read_bypasses_backoff(kind) {
            return None;
        }

        let mut backoff = self
            .feedback_read_backoff
            .lock()
            .expect("feedback read backoff mutex");
        let key = (client_id.to_string(), kind);
        let now = Instant::now();
        match backoff.get(&key).copied() {
            Some(state) if state.blocked_until > now => {
                Some(state.blocked_until.saturating_duration_since(now))
            }
            Some(_) => {
                backoff.remove(&key);
                None
            }
            None => None,
        }
    }

    fn record_feedback_read_success(&self, client_id: &str, kind: DownloadFeedbackReadKind) {
        let mut backoff = self
            .feedback_read_backoff
            .lock()
            .expect("feedback read backoff mutex");
        backoff.remove(&(client_id.to_string(), kind));
    }

    fn record_feedback_read_failure(&self, client_id: &str, kind: DownloadFeedbackReadKind) {
        let mut backoff = self
            .feedback_read_backoff
            .lock()
            .expect("feedback read backoff mutex");
        let key = (client_id.to_string(), kind);
        let failures = backoff
            .get(&key)
            .map(|state| state.consecutive_failures.saturating_add(1))
            .unwrap_or(1);
        let delay = Self::feedback_backoff_duration(failures);
        backoff.insert(
            key,
            FeedbackReadBackoffState {
                consecutive_failures: failures,
                blocked_until: Instant::now() + delay,
            },
        );
    }

    async fn list_enabled_clients_by_priority(&self) -> AppResult<Vec<DownloadClientConfig>> {
        let mut clients = self
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|config| config.is_enabled)
            .collect::<Vec<_>>();
        clients.sort_by_key(|config| config.client_priority);
        Ok(clients)
    }

    fn request_source_kind(request: &DownloadClientAddRequest) -> Option<DownloadSourceKind> {
        request
            .source_kind
            .or_else(|| DownloadSourceKind::infer_from_hint(request.source_hint.as_deref()))
            .or_else(|| {
                request
                    .info_hash_hint
                    .as_ref()
                    .map(|_| DownloadSourceKind::TorrentFile)
            })
    }

    fn source_kind_label(kind: DownloadSourceKind) -> &'static str {
        match kind {
            DownloadSourceKind::NzbFile => "NZB file",
            DownloadSourceKind::NzbUrl => "NZB URL",
            DownloadSourceKind::TorrentFile => "torrent file",
            DownloadSourceKind::MagnetUri => "magnet",
        }
    }

    fn config_accepts_source_kind(
        config: &DownloadClientConfig,
        source_kind: DownloadSourceKind,
        plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
    ) -> bool {
        let accepted_inputs = accepted_inputs_for_client(&config.client_type, plugin_provider);
        if accepted_inputs.is_empty() {
            return false;
        }
        accepted_inputs.iter().any(|&accepted_kind| {
            // NzbFile and NzbUrl are interchangeable — scryer fetches the URL
            // and sends the file content, so any NZB-capable client handles both.
            match (accepted_kind, source_kind) {
                (DownloadSourceKind::NzbFile, DownloadSourceKind::NzbUrl)
                | (DownloadSourceKind::NzbUrl, DownloadSourceKind::NzbFile) => true,
                _ => accepted_kind == source_kind,
            }
        })
    }

    fn read_trimmed_string(raw_value: Option<&serde_json::Value>) -> Option<String> {
        raw_value
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    fn read_bool(raw_value: Option<&serde_json::Value>, default: bool) -> bool {
        match raw_value {
            Some(serde_json::Value::Bool(value)) => *value,
            Some(serde_json::Value::String(value)) => !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "false" | "0" | "no"
            ),
            Some(serde_json::Value::Number(value)) => value.as_i64() != Some(0),
            _ => default,
        }
    }

    fn parse_routing_object(raw_json: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
        serde_json::from_str::<serde_json::Value>(raw_json)
            .ok()?
            .as_object()
            .cloned()
    }

    fn parse_routing_entry(config: &serde_json::Value) -> DownloadClientRoutingEntry {
        DownloadClientRoutingEntry {
            enabled: Self::read_bool(config.get("enabled"), true),
            category: Self::read_trimmed_string(config.get("category")),
            recent_queue_priority: Self::read_trimmed_string(
                config
                    .get("recentQueuePriority")
                    .or_else(|| config.get("recentPriority"))
                    .or_else(|| config.get("recent_priority")),
            ),
            older_queue_priority: Self::read_trimmed_string(
                config
                    .get("olderQueuePriority")
                    .or_else(|| config.get("olderPriority"))
                    .or_else(|| config.get("older_priority")),
            ),
            remove_completed: Self::read_bool(
                config
                    .get("removeCompleted")
                    .or_else(|| config.get("remove_completed"))
                    .or_else(|| config.get("removeComplete")),
                false,
            ),
            remove_failed: Self::read_bool(
                config
                    .get("removeFailed")
                    .or_else(|| config.get("remove_failed"))
                    .or_else(|| config.get("removeFailure")),
                false,
            ),
        }
    }

    fn facet_scope_id(facet: &MediaFacet) -> &'static str {
        facet.as_str()
    }

    async fn get_download_client_routing_json(&self, scope_id: &str) -> AppResult<Option<String>> {
        if let Some(routing_json) = self
            .settings
            .get_setting_json(
                "system",
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
            )
            .await?
        {
            return Ok(Some(routing_json));
        }

        self.settings
            .get_setting_json(
                "system",
                LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
            )
            .await
    }

    async fn get_explicit_download_client_routing_json(
        &self,
        scope_id: &str,
    ) -> AppResult<Option<String>> {
        if let Some(routing_json) = self
            .settings
            .get_setting_json_explicit(
                "system",
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
            )
            .await?
        {
            return Ok(Some(routing_json));
        }

        self.settings
            .get_setting_json_explicit(
                "system",
                LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
            )
            .await
    }

    async fn resolve_routing_object_for_title(
        &self,
        title: &scryer_domain::Title,
    ) -> AppResult<Option<ResolvedDownloadClientRouting>> {
        if let Some(raw_json) = self
            .get_explicit_download_client_routing_json(&title.library_id)
            .await?
        {
            if let Some(routing_object) = Self::parse_routing_object(&raw_json) {
                return Ok(Some(ResolvedDownloadClientRouting {
                    scope: DownloadClientRoutingScope::Library,
                    routing_object,
                }));
            }

            warn!(
                library_id = title.library_id.as_str(),
                title = title.name.as_str(),
                "ignoring invalid library-scoped download client routing override"
            );
        }

        let scope_id = Self::facet_scope_id(&title.facet);
        if let Some(raw_json) = self.get_download_client_routing_json(scope_id).await? {
            if let Some(routing_object) = Self::parse_routing_object(&raw_json) {
                return Ok(Some(ResolvedDownloadClientRouting {
                    scope: DownloadClientRoutingScope::Facet,
                    routing_object,
                }));
            }

            warn!(
                facet = ?title.facet,
                title = title.name.as_str(),
                "ignoring invalid facet-scoped download client routing settings"
            );
        }

        Ok(None)
    }

    /// Return enabled clients ordered by effective routing priority for this title.
    /// Falls back to global `client_priority` if no routing config applies.
    async fn list_clients_for_title(
        &self,
        title: &scryer_domain::Title,
    ) -> AppResult<FacetClientSelection> {
        let resolved_routing = self.resolve_routing_object_for_title(title).await?;

        let mut clients = self
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|config| config.is_enabled)
            .collect::<Vec<_>>();
        let any_globally_enabled = !clients.is_empty();
        let mut disabled_scope = None;

        match resolved_routing {
            Some(resolved_routing) => {
                let ordered_ids: Vec<String> =
                    resolved_routing.routing_object.keys().cloned().collect();
                let missing_client_default_enabled =
                    !matches!(resolved_routing.scope, DownloadClientRoutingScope::Library);

                clients.retain(|client| {
                    resolved_routing
                        .routing_object
                        .get(&client.id)
                        .map(|entry| Self::read_bool(entry.get("enabled"), true))
                        .unwrap_or(missing_client_default_enabled)
                });

                if any_globally_enabled && clients.is_empty() {
                    disabled_scope = Some(resolved_routing.scope);
                }

                if ordered_ids.is_empty() {
                    clients.sort_by_key(|c| c.client_priority);
                } else {
                    clients.sort_by_key(|c| {
                        ordered_ids
                            .iter()
                            .position(|id| id == &c.id)
                            .unwrap_or(usize::MAX)
                    });
                }
            }
            None => {
                clients.sort_by_key(|c| c.client_priority);
            }
        }

        Ok(FacetClientSelection {
            clients,
            disabled_scope,
        })
    }

    async fn routing_entry_for_client(
        &self,
        title: &scryer_domain::Title,
        client_id: &str,
    ) -> AppResult<Option<DownloadClientRoutingEntry>> {
        let Some(resolved_routing) = self.resolve_routing_object_for_title(title).await? else {
            return Ok(None);
        };

        Ok(resolved_routing
            .routing_object
            .get(client_id)
            .map(Self::parse_routing_entry))
    }

    fn normalized_request_category(request: &DownloadClientAddRequest) -> Option<String> {
        request
            .category
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    async fn apply_selected_client_routing(
        &self,
        request: &DownloadClientAddRequest,
        client_id: &str,
    ) -> AppResult<DownloadClientAddRequest> {
        let mut effective_request = request.clone();
        let routing_entry = self
            .routing_entry_for_client(&request.title, client_id)
            .await?;

        effective_request.category = routing_entry
            .as_ref()
            .and_then(|entry| entry.category.clone())
            .or_else(|| Self::normalized_request_category(request));

        let is_recent = request.is_recent.unwrap_or(false);
        effective_request.queue_priority = routing_entry.and_then(|entry| {
            if is_recent {
                entry.recent_queue_priority
            } else {
                entry.older_queue_priority
            }
        });

        Ok(effective_request)
    }

    fn is_native_nzb_client_type(client_type: &str) -> bool {
        matches!(client_type, "nzbget" | "sabnzbd" | "weaver")
    }

    fn request_uses_nzb_payload(request: &DownloadClientAddRequest) -> bool {
        matches!(
            Self::request_source_kind(request),
            Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl)
        )
    }

    async fn delete_staged_nzb(&self, staged_nzb: Option<&StagedNzbRef>, reason: &str) {
        let Some(staged_nzb) = staged_nzb else {
            return;
        };

        if let Err(error) = self.staged_nzb_store.delete_staged_nzb(staged_nzb).await {
            warn!(
                staged_nzb_id = staged_nzb.id.as_str(),
                error = %error,
                reason,
                "failed to delete staged nzb artifact"
            );
        }
    }

    fn client_from_config(
        config: &DownloadClientConfig,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
        plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
        feedback_read_timeout: Duration,
    ) -> AppResult<Arc<dyn DownloadClient>> {
        if let Some(provider) = plugin_provider
            && let Some(client) = provider.client_for_config(config)
        {
            return Ok(Self::wrap_feedback_client(client, feedback_read_timeout));
        }

        match config.client_type.as_str() {
            "nzbget" => {
                let parsed_config = parse_download_client_config_json(&config.config_json)?;
                let base_url =
                    resolve_download_client_base_url(&parsed_config).ok_or_else(|| {
                        AppError::Validation(format!(
                            "download client {} has no valid base URL",
                            config.id
                        ))
                    })?;
                let username = read_config_string(&parsed_config, &["username"]);
                let password = read_config_string(&parsed_config, &["password"]);
                let dupe_mode = read_config_string(&parsed_config, &["dupe_mode", "dupeMode"])
                    .unwrap_or_else(|| "SCORE".to_string());
                let client = NzbgetDownloadClient::with_staged_nzb_store(
                    base_url,
                    username,
                    password,
                    dupe_mode,
                    staged_nzb_store,
                    staged_nzb_pipeline_limit,
                );
                Ok(Self::wrap_feedback_client(
                    Arc::new(client),
                    feedback_read_timeout,
                ))
            }
            "sabnzbd" => {
                let parsed_config = parse_download_client_config_json(&config.config_json)?;
                let base_url =
                    resolve_download_client_base_url(&parsed_config).ok_or_else(|| {
                        AppError::Validation(format!(
                            "download client {} has no valid base URL",
                            config.id
                        ))
                    })?;
                let api_key = read_config_string(&parsed_config, &["api_key", "apiKey", "apikey"]);
                let username = read_config_string(&parsed_config, &["username"]);
                let password = read_config_string(&parsed_config, &["password"]);
                if api_key.is_none() && (username.is_none() || password.is_none()) {
                    return Err(AppError::Validation(format!(
                        "download client {} (sabnzbd) requires an API key or username/password",
                        config.id
                    )));
                }
                let client = SabnzbdDownloadClient::with_auth_and_staged_nzb_store(
                    base_url,
                    api_key,
                    username,
                    password,
                    staged_nzb_store,
                    staged_nzb_pipeline_limit,
                );
                Ok(Self::wrap_feedback_client(
                    Arc::new(client),
                    feedback_read_timeout,
                ))
            }
            "weaver" => {
                let client = WeaverDownloadClient::from_config_with_staged_nzb_store(
                    config,
                    staged_nzb_store,
                    staged_nzb_pipeline_limit,
                )?;
                Ok(Self::wrap_feedback_client(
                    Arc::new(client),
                    feedback_read_timeout,
                ))
            }
            _ => Err(AppError::Validation(format!(
                "unsupported download client type '{}' for config {}",
                config.client_type, config.id
            ))),
        }
    }

    async fn resolve_client_for_queue_action(
        &self,
        id: &str,
        is_history: bool,
    ) -> AppResult<Option<Arc<dyn DownloadClient>>> {
        let configs = self.list_enabled_clients_by_priority().await?;
        if configs.is_empty() {
            return Ok(None);
        }

        let mut clients = Vec::new();
        for config in configs {
            match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => clients.push((config, client)),
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        "download client skipped while routing queue action"
                    );
                }
            }
        }

        if clients.is_empty() {
            return Ok(None);
        }

        for (config, client) in &clients {
            let items = if is_history {
                client.list_history().await
            } else {
                client.list_queue().await
            };

            match items {
                Ok(items) => {
                    if items.iter().any(|item| item.download_client_item_id == id) {
                        return Ok(Some(Arc::clone(client)));
                    }
                }
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        queue_item_id = id,
                        history = is_history,
                        error = %error,
                        "failed to inspect download client while routing queue action"
                    );
                }
            }
        }

        if clients.len() == 1 {
            return Ok(Some(Arc::clone(&clients[0].1)));
        }

        Err(AppError::Validation(format!(
            "download client item not found: {id}"
        )))
    }

    async fn resolve_client_for_id(
        &self,
        client_id: &str,
    ) -> AppResult<Option<Arc<dyn DownloadClient>>> {
        let normalized = client_id.trim();
        if normalized.is_empty() {
            return Ok(None);
        }

        let configs = self.list_enabled_clients_by_priority().await?;
        for config in configs {
            if config.id != normalized {
                continue;
            }

            return Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            )
            .map(Some);
        }

        Ok(None)
    }

    async fn resolve_client_for_type(
        &self,
        client_type: &str,
    ) -> AppResult<Option<Arc<dyn DownloadClient>>> {
        let normalized = client_type.trim();
        if normalized.is_empty() {
            return Ok(None);
        }

        let configs = self.list_enabled_clients_by_priority().await?;
        if configs.is_empty() {
            return Ok(None);
        }

        for config in configs {
            if !config.client_type.eq_ignore_ascii_case(normalized) {
                continue;
            }

            return Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            )
            .map(Some);
        }

        Ok(None)
    }
}

#[async_trait]
impl DownloadClient for PrioritizedDownloadClientRouter {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        let selection = match self.list_clients_for_title(&request.title).await {
            Ok(configs) => configs,
            Err(error) => {
                warn!(
                    error = %error,
                    title = request.title.name.as_str(),
                    facet = ?request.title.facet,
                    "failed to load prioritized download clients; falling back to default client"
                );
                return self.fallback_client.submit_download(request).await;
            }
        };

        if let Some(disabled_scope) = selection.disabled_scope {
            let message = match disabled_scope {
                DownloadClientRoutingScope::Library => format!(
                    "no download client enabled for library {}",
                    request.title.library_id
                ),
                DownloadClientRoutingScope::Facet => {
                    "no download client enabled for this facet".to_string()
                }
            };
            return Err(AppError::Validation(message));
        }

        let mut clients = selection.clients;

        if clients.is_empty() {
            return self.fallback_client.submit_download(request).await;
        }

        if let Some(source_kind) = Self::request_source_kind(request) {
            clients.retain(|config| {
                let compatible = Self::config_accepts_source_kind(
                    config,
                    source_kind,
                    self.plugin_provider.as_ref(),
                );
                if !compatible {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        source_kind = source_kind.as_str(),
                        "download client skipped because it cannot handle this release type"
                    );
                }
                compatible
            });

            if clients.is_empty() {
                return Err(AppError::Validation(format!(
                    "no enabled download client can handle {} releases",
                    Self::source_kind_label(source_kind)
                )));
            }
        }

        let mut last_error: Option<AppError> = None;
        let mut staged_nzb = if let Some(staged_nzb) = request.staged_nzb.clone() {
            self.staged_nzb_store
                .mark_artifact_active(&staged_nzb.compressed_path)?;
            Some(super::StagedNzbLease {
                staged_nzb,
                self_staged: false,
                store: self.staged_nzb_store.clone(),
                _permit: None,
            })
        } else {
            None
        };
        for config in clients {
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        "download client skipped due to invalid configuration"
                    );
                    last_error = Some(error);
                    continue;
                }
            };

            let effective_request = match self
                .apply_selected_client_routing(request, &config.id)
                .await
            {
                Ok(mut effective_request) => {
                    if Self::is_native_nzb_client_type(&config.client_type)
                        && Self::request_uses_nzb_payload(&effective_request)
                    {
                        if staged_nzb.is_none() {
                            let source_hint = request_source_hint_for_nzb(&effective_request)?;
                            staged_nzb = Some(
                                stage_nzb_from_url(
                                    &self.outbound_http,
                                    &self.staged_nzb_store,
                                    &self.staged_nzb_pipeline_limit,
                                    &source_hint,
                                    Some(&request.title.id),
                                )
                                .await?,
                            );
                        }
                        effective_request.staged_nzb =
                            staged_nzb.as_ref().map(|lease| lease.staged_nzb.clone());
                    }
                    effective_request
                }
                Err(error) => {
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        "download client skipped because routing configuration could not be resolved"
                    );
                    last_error = Some(error);
                    continue;
                }
            };

            match client.submit_download(&effective_request).await {
                Ok(result) => {
                    self.delete_staged_nzb(
                        staged_nzb.as_ref().map(|lease| &lease.staged_nzb),
                        "submit_success",
                    )
                    .await;
                    return Ok(DownloadGrabResult {
                        job_id: result.job_id,
                        client_id: Some(config.id.clone()),
                        client_type: config.client_type.clone(),
                        info_hash: result.info_hash,
                    });
                }
                Err(error) => {
                    let should_failover = matches!(
                        error,
                        AppError::Repository(_) | AppError::DownloadSubmitUnavailable(_)
                    );
                    warn!(
                        client_id = config.id.as_str(),
                        client_name = config.name.as_str(),
                        client_type = config.client_type.as_str(),
                        error = %error,
                        failover = should_failover,
                        "download client enqueue failed"
                    );
                    if should_failover {
                        last_error = Some(error);
                        continue;
                    }
                    self.delete_staged_nzb(
                        staged_nzb.as_ref().map(|lease| &lease.staged_nzb),
                        "submit_failure",
                    )
                    .await;
                    return Err(error);
                }
            }
        }

        self.delete_staged_nzb(
            staged_nzb.as_ref().map(|lease| &lease.staged_nzb),
            "submit_failure",
        )
        .await;

        Err(last_error
            .unwrap_or_else(|| {
                AppError::Repository(
                    "all prioritized download clients failed to enqueue this release".to_string(),
                )
            })
            .into_download_submit_unavailable())
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self.fallback_client.list_queue().await;
        }
        let mut all_items = Vec::new();
        let mut read_summary = FeedbackReadSummary::default();
        for config in clients {
            if let Some(remaining) =
                self.feedback_backoff_remaining(&config.id, DownloadFeedbackReadKind::Queue)
            {
                debug!(
                    client_id = %config.id,
                    client = %config.name,
                    read_kind = Self::feedback_read_kind_label(DownloadFeedbackReadKind::Queue),
                    remaining_ms = remaining.as_millis(),
                    "skipping download client feedback read during backoff"
                );
                continue;
            }
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for queue listing");
                    continue;
                }
            };
            match client.list_queue().await {
                Ok(mut items) => {
                    self.record_feedback_read_success(&config.id, DownloadFeedbackReadKind::Queue);
                    read_summary.record_success();
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(&config.id, DownloadFeedbackReadKind::Queue);
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list queue");
                }
            }
        }
        read_summary.finish()?;
        Ok(all_items)
    }

    async fn list_queue_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self.fallback_client.list_queue_for_title(title_id).await;
        }
        let mut all_items = Vec::new();
        let mut read_summary = FeedbackReadSummary::default();
        for config in clients {
            let _ =
                self.feedback_backoff_remaining(&config.id, DownloadFeedbackReadKind::TitleQueue);
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for title-scoped queue listing");
                    continue;
                }
            };
            match client.list_queue_for_title(title_id).await {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::TitleQueue,
                    );
                    read_summary.record_success();
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::TitleQueue,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list title-scoped queue");
                }
            }
        }
        read_summary.finish()?;
        Ok(all_items)
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self.fallback_client.list_history().await;
        }
        let mut all_items = Vec::new();
        let mut read_summary = FeedbackReadSummary::default();
        for config in clients {
            if let Some(remaining) =
                self.feedback_backoff_remaining(&config.id, DownloadFeedbackReadKind::History)
            {
                debug!(
                    client_id = %config.id,
                    client = %config.name,
                    read_kind = Self::feedback_read_kind_label(DownloadFeedbackReadKind::History),
                    remaining_ms = remaining.as_millis(),
                    "skipping download client feedback read during backoff"
                );
                continue;
            }
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for history listing");
                    continue;
                }
            };
            match client.list_history().await {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::History,
                    );
                    read_summary.record_success();
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::History,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list history");
                }
            }
        }
        read_summary.finish()?;
        Ok(all_items)
    }

    async fn list_recent_activity(&self, limit: usize) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self.fallback_client.list_recent_activity(limit).await;
        }

        let mut all_items = Vec::new();
        let mut read_summary = FeedbackReadSummary::default();
        for config in clients {
            if let Some(remaining) = self
                .feedback_backoff_remaining(&config.id, DownloadFeedbackReadKind::RecentActivity)
            {
                debug!(
                    client_id = %config.id,
                    client = %config.name,
                    read_kind = Self::feedback_read_kind_label(
                        DownloadFeedbackReadKind::RecentActivity
                    ),
                    remaining_ms = remaining.as_millis(),
                    "skipping download client feedback read during backoff"
                );
                continue;
            }
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for recent activity listing");
                    continue;
                }
            };
            match client.list_recent_activity(limit).await {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::RecentActivity,
                    );
                    read_summary.record_success();
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::RecentActivity,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list recent activity");
                }
            }
        }

        read_summary.finish()?;

        let mut seen = HashSet::with_capacity(all_items.len());
        all_items.retain(|item| seen.insert(download_queue_history_key(item)));
        all_items.sort_by(compare_history_items_desc);
        all_items.truncate(limit);
        Ok(all_items)
    }

    async fn list_recent_activity_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self
                .fallback_client
                .list_recent_activity_for_title(title_id, limit)
                .await;
        }

        let mut all_items = Vec::new();
        let mut read_summary = FeedbackReadSummary::default();
        for config in clients {
            let _ = self.feedback_backoff_remaining(
                &config.id,
                DownloadFeedbackReadKind::TitleRecentActivity,
            );
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for title-scoped recent activity listing");
                    continue;
                }
            };
            match client.list_recent_activity_for_title(title_id, limit).await {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::TitleRecentActivity,
                    );
                    read_summary.record_success();
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::TitleRecentActivity,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list title-scoped recent activity");
                }
            }
        }

        read_summary.finish()?;

        let mut seen = HashSet::with_capacity(all_items.len());
        all_items.retain(|item| seen.insert(download_queue_history_key(item)));
        all_items.sort_by(compare_history_items_desc);
        all_items.truncate(limit);
        Ok(all_items)
    }

    async fn list_history_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self.fallback_client.list_history_page(offset, limit).await;
        }

        let fetch_limit = offset.saturating_add(limit);
        let mut all_items = Vec::new();
        let mut read_summary = FeedbackReadSummary::default();
        for config in clients {
            if let Some(remaining) =
                self.feedback_backoff_remaining(&config.id, DownloadFeedbackReadKind::History)
            {
                debug!(
                    client_id = %config.id,
                    client = %config.name,
                    read_kind = Self::feedback_read_kind_label(DownloadFeedbackReadKind::History),
                    remaining_ms = remaining.as_millis(),
                    "skipping download client feedback read during backoff"
                );
                continue;
            }
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for paged history listing");
                    continue;
                }
            };
            match client.list_history_page(0, fetch_limit).await {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::History,
                    );
                    read_summary.record_success();
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        item.client_name = config.name.clone();
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::History,
                    );
                    read_summary.record_error(&error);
                    tracing::warn!(client_id = %config.id, error = %error, "failed to list paged history");
                }
            }
        }

        read_summary.finish()?;

        let mut seen = HashSet::with_capacity(all_items.len());
        all_items.retain(|item| seen.insert(download_queue_history_key(item)));
        all_items.sort_by(compare_history_items_desc);

        Ok(all_items.into_iter().skip(offset).take(limit).collect())
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self.fallback_client.list_completed_downloads().await;
        }
        let mut all_items = Vec::new();
        for config in clients {
            if let Some(remaining) = self.feedback_backoff_remaining(
                &config.id,
                DownloadFeedbackReadKind::RecentCompletedDownloads,
            ) {
                debug!(
                    client_id = %config.id,
                    client = %config.name,
                    read_kind = Self::feedback_read_kind_label(
                        DownloadFeedbackReadKind::RecentCompletedDownloads
                    ),
                    remaining_ms = remaining.as_millis(),
                    "skipping download client feedback read during backoff"
                );
                continue;
            }
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for completed downloads");
                    continue;
                }
            };
            match client.list_completed_downloads().await {
                Ok(mut items) => {
                    tracing::debug!(
                        client = %config.name,
                        client_type = %config.client_type,
                        count = items.len(),
                        "completed downloads from client"
                    );
                    let mappings = download_client_remote_path_mappings(&config);
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        if let Some(mappings) = mappings.as_deref() {
                            apply_remote_path_mappings_to_completed_download(item, mappings);
                        }
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    tracing::warn!(client_id = %config.id, client = %config.name, error = %error, "failed to list completed downloads");
                }
            }
        }
        Ok(all_items)
    }

    async fn list_recent_completed_downloads(
        &self,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let clients = self.list_enabled_clients_by_priority().await?;
        if clients.is_empty() {
            return self
                .fallback_client
                .list_recent_completed_downloads(limit)
                .await;
        }

        let mut all_items = Vec::new();
        for config in clients {
            let client = match Self::client_from_config(
                &config,
                self.staged_nzb_store.clone(),
                self.staged_nzb_pipeline_limit.clone(),
                self.plugin_provider.as_ref(),
                self.feedback_read_timeout,
            ) {
                Ok(client) => client,
                Err(error) => {
                    tracing::warn!(client_id = %config.id, error = %error, "skipping client for recent completed downloads");
                    continue;
                }
            };
            match client.list_recent_completed_downloads(limit).await {
                Ok(mut items) => {
                    self.record_feedback_read_success(
                        &config.id,
                        DownloadFeedbackReadKind::RecentCompletedDownloads,
                    );
                    tracing::debug!(
                        client = %config.name,
                        client_type = %config.client_type,
                        count = items.len(),
                        "recent completed downloads from client"
                    );
                    let mappings = download_client_remote_path_mappings(&config);
                    for item in &mut items {
                        item.client_id = config.id.clone();
                        if let Some(mappings) = mappings.as_deref() {
                            apply_remote_path_mappings_to_completed_download(item, mappings);
                        }
                    }
                    all_items.extend(items);
                }
                Err(error) => {
                    self.record_feedback_read_failure(
                        &config.id,
                        DownloadFeedbackReadKind::RecentCompletedDownloads,
                    );
                    tracing::warn!(client_id = %config.id, client = %config.name, error = %error, "failed to list recent completed downloads");
                }
            }
        }

        all_items.sort_by(compare_completed_downloads_desc);
        all_items.truncate(limit);
        Ok(all_items)
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_queue_action(id, false).await? {
            return client.pause_queue_item(id).await;
        }
        self.fallback_client.pause_queue_item(id).await
    }

    async fn pause_queue_item_for_client(&self, client_id: &str, id: &str) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_id(client_id).await? {
            return client.pause_queue_item(id).await;
        }
        Err(AppError::Validation(format!(
            "download client not found: {client_id}"
        )))
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_queue_action(id, false).await? {
            return client.resume_queue_item(id).await;
        }
        self.fallback_client.resume_queue_item(id).await
    }

    async fn resume_queue_item_for_client(&self, client_id: &str, id: &str) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_id(client_id).await? {
            return client.resume_queue_item(id).await;
        }
        Err(AppError::Validation(format!(
            "download client not found: {client_id}"
        )))
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_queue_action(id, is_history).await? {
            return client.delete_queue_item(id, is_history).await;
        }
        self.fallback_client.delete_queue_item(id, is_history).await
    }

    async fn delete_queue_item_for_client_id(
        &self,
        client_id: &str,
        id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_id(client_id).await? {
            return client.delete_queue_item(id, is_history).await;
        }
        Err(AppError::Validation(format!(
            "download client not found: {client_id}"
        )))
    }

    async fn delete_queue_item_for_client(
        &self,
        client_type: &str,
        id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        if let Some(client) = self.resolve_client_for_type(client_type).await? {
            return client.delete_queue_item(id, is_history).await;
        }
        self.fallback_client
            .delete_queue_item_for_client(client_type, id, is_history)
            .await
    }

    async fn get_client_status_for_client_id(
        &self,
        client_id: &str,
    ) -> AppResult<DownloadClientStatus> {
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err(AppError::Validation("client id is required".into()));
        }

        let config = self
            .download_client_configs
            .get_by_id(client_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!("download client not found: {client_id}"))
            })?;
        let client = Self::client_from_config(
            &config,
            self.staged_nzb_store.clone(),
            self.staged_nzb_pipeline_limit.clone(),
            self.plugin_provider.as_ref(),
            self.feedback_read_timeout,
        )?;
        let mut status = client.get_client_status().await?;
        let mappings = download_client_remote_path_mappings(&config);
        if let Some(mappings) = mappings.as_deref() {
            apply_remote_path_mappings_to_status(&mut status, mappings);
        }
        Ok(status)
    }
}

fn parse_history_timestamp(value: Option<&str>) -> i64 {
    value
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0)
}

fn compare_history_items_desc(
    left: &DownloadQueueItem,
    right: &DownloadQueueItem,
) -> std::cmp::Ordering {
    parse_history_timestamp(right.last_updated_at.as_deref())
        .cmp(&parse_history_timestamp(left.last_updated_at.as_deref()))
        .then_with(|| right.id.cmp(&left.id))
}

fn download_queue_history_key(item: &DownloadQueueItem) -> String {
    if item.client_type.is_empty() && item.download_client_item_id.is_empty() {
        return item.id.clone();
    }

    if item.client_id.trim().is_empty() {
        return format!("{}:{}", item.client_type, item.download_client_item_id);
    }

    format!("{}:{}", item.client_id, item.download_client_item_id)
}

fn compare_completed_downloads_desc(
    left: &scryer_domain::CompletedDownload,
    right: &scryer_domain::CompletedDownload,
) -> std::cmp::Ordering {
    right.completed_at.cmp(&left.completed_at).then_with(|| {
        right
            .download_client_item_id
            .cmp(&left.download_client_item_id)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    struct MockDownloadClientConfigRepository {
        configs: Vec<DownloadClientConfig>,
    }

    #[async_trait]
    impl DownloadClientConfigRepository for MockDownloadClientConfigRepository {
        async fn list(
            &self,
            _provider_type: Option<String>,
        ) -> AppResult<Vec<DownloadClientConfig>> {
            Ok(self.configs.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
            Ok(self.configs.iter().find(|config| config.id == id).cloned())
        }

        async fn create(&self, _config: DownloadClientConfig) -> AppResult<DownloadClientConfig> {
            unreachable!("not used in router tests")
        }

        async fn update(
            &self,
            _update: scryer_application::DownloadClientConfigUpdate,
        ) -> AppResult<DownloadClientConfig> {
            unreachable!("not used in router tests")
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            unreachable!("not used in router tests")
        }

        async fn reorder(&self, _ordered_ids: Vec<String>) -> AppResult<()> {
            unreachable!("not used in router tests")
        }
    }

    #[derive(Default)]
    struct MockSettingsRepository {
        routing_by_scope: HashMap<String, String>,
    }

    #[async_trait]
    impl SettingsRepository for MockSettingsRepository {
        async fn get_setting_json(
            &self,
            _scope: &str,
            _key_name: &str,
            scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            Ok(scope_id.and_then(|id| self.routing_by_scope.get(&id).cloned()))
        }

        async fn get_setting_json_explicit(
            &self,
            _scope: &str,
            _key_name: &str,
            scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            Ok(scope_id.and_then(|id| self.routing_by_scope.get(&id).cloned()))
        }

        async fn upsert_setting_json(
            &self,
            _scope: &str,
            _key_name: &str,
            _scope_id: Option<String>,
            _value_json: String,
            _source: &str,
            _updated_by_user_id: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn delete_setting_value(
            &self,
            _scope: &str,
            _key_name: &str,
            _scope_id: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn delete_values_for_scope_id(&self, _scope_id: &str) -> AppResult<u32> {
            Ok(0)
        }
    }

    #[derive(Default)]
    struct MockDownloadClient {
        submissions: Mutex<Vec<DownloadClientAddRequest>>,
        submit_error: Mutex<Option<MockSubmitError>>,
        queue_items: Mutex<Vec<DownloadQueueItem>>,
        history_items: Mutex<Vec<DownloadQueueItem>>,
        completed_downloads: Mutex<Vec<scryer_domain::CompletedDownload>>,
        status: Mutex<DownloadClientStatus>,
        paused: Mutex<Vec<String>>,
        resumed: Mutex<Vec<String>>,
        deleted: Mutex<Vec<(String, bool)>>,
    }

    #[derive(Clone, Copy)]
    enum MockSubmitError {
        Ambiguous,
        Repository,
        SubmitUnavailable,
    }

    #[async_trait]
    impl DownloadClient for MockDownloadClient {
        async fn submit_download(
            &self,
            request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            self.submissions.lock().unwrap().push(request.clone());
            match *self.submit_error.lock().unwrap() {
                Some(MockSubmitError::Ambiguous) => {
                    return Err(AppError::DownloadSubmitAmbiguous(
                        "submit result is ambiguous".to_string(),
                    ));
                }
                Some(MockSubmitError::Repository) => {
                    return Err(AppError::Repository("client enqueue failed".to_string()));
                }
                Some(MockSubmitError::SubmitUnavailable) => {
                    return Err(AppError::download_submit_unavailable(
                        "client submit unavailable",
                    ));
                }
                None => {}
            }
            Ok(DownloadGrabResult {
                job_id: "job-1".to_string(),
                client_id: None,
                client_type: "mock".to_string(),
                info_hash: None,
            })
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            Ok(self.queue_items.lock().unwrap().clone())
        }

        async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
            Ok(self.history_items.lock().unwrap().clone())
        }

        async fn list_completed_downloads(
            &self,
        ) -> AppResult<Vec<scryer_domain::CompletedDownload>> {
            Ok(self.completed_downloads.lock().unwrap().clone())
        }

        async fn get_client_status(&self) -> AppResult<DownloadClientStatus> {
            Ok(self.status.lock().unwrap().clone())
        }

        async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
            self.paused.lock().unwrap().push(id.to_string());
            Ok(())
        }

        async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
            self.resumed.lock().unwrap().push(id.to_string());
            Ok(())
        }

        async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
            self.deleted
                .lock()
                .unwrap()
                .push((id.to_string(), is_history));
            Ok(())
        }
    }

    struct MockDownloadClientPluginProvider {
        accepted_inputs: Vec<String>,
        clients: Vec<(String, Arc<dyn DownloadClient>)>,
    }

    impl DownloadClientPluginProvider for MockDownloadClientPluginProvider {
        fn client_for_config(
            &self,
            config: &DownloadClientConfig,
        ) -> Option<Arc<dyn DownloadClient>> {
            self.clients
                .iter()
                .find(|(id, _)| id == &config.id)
                .map(|(_, client)| Arc::clone(client))
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["qbittorrent".to_string()]
        }

        fn accepted_inputs_for_provider(&self, _provider_type: &str) -> Vec<String> {
            self.accepted_inputs.clone()
        }
    }

    struct DelayedQueueDownloadClient {
        delay: Duration,
        queue_items: Vec<DownloadQueueItem>,
    }

    #[async_trait]
    impl DownloadClient for DelayedQueueDownloadClient {
        async fn submit_download(
            &self,
            _request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Ok(DownloadGrabResult {
                job_id: "job-1".to_string(),
                client_id: None,
                client_type: "delayed".to_string(),
                info_hash: None,
            })
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            tokio::time::sleep(self.delay).await;
            Ok(self.queue_items.clone())
        }
    }

    #[derive(Default)]
    struct FailingQueueDownloadClient {
        list_queue_calls: AtomicUsize,
        list_queue_for_title_calls: AtomicUsize,
    }

    impl FailingQueueDownloadClient {
        fn list_queue_call_count(&self) -> usize {
            self.list_queue_calls.load(Ordering::SeqCst)
        }

        fn list_queue_for_title_call_count(&self) -> usize {
            self.list_queue_for_title_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl DownloadClient for FailingQueueDownloadClient {
        async fn submit_download(
            &self,
            _request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Ok(DownloadGrabResult {
                job_id: "job-1".to_string(),
                client_id: None,
                client_type: "failing".to_string(),
                info_hash: None,
            })
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            self.list_queue_calls.fetch_add(1, Ordering::SeqCst);
            Err(AppError::Repository("queue unavailable".to_string()))
        }

        async fn list_queue_for_title(&self, _title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
            self.list_queue_for_title_calls
                .fetch_add(1, Ordering::SeqCst);
            Err(AppError::Repository("title queue unavailable".to_string()))
        }
    }

    fn test_title_for_facet(facet: MediaFacet) -> scryer_domain::Title {
        scryer_domain::Title {
            id: "title-1".to_string(),
            name: "Test Title".to_string(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            facet,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
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
            genres: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn test_title() -> scryer_domain::Title {
        test_title_for_facet(MediaFacet::Movie)
    }

    fn test_config(id: &str, name: &str, client_type: &str, priority: i64) -> DownloadClientConfig {
        DownloadClientConfig {
            id: id.to_string(),
            name: name.to_string(),
            client_type: client_type.to_string(),
            config_json: "{}".to_string(),
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            client_priority: priority,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn null_staged_nzb_store() -> Arc<dyn StagedNzbStore> {
        Arc::new(scryer_application::NullStagedNzbStore)
    }

    fn test_pipeline_limit() -> Arc<Semaphore> {
        Arc::new(Semaphore::new(4))
    }

    fn disabled_test_config(
        id: &str,
        name: &str,
        client_type: &str,
        priority: i64,
    ) -> DownloadClientConfig {
        DownloadClientConfig {
            is_enabled: false,
            ..test_config(id, name, client_type, priority)
        }
    }

    fn test_queue_item(id: &str) -> DownloadQueueItem {
        DownloadQueueItem {
            id: format!("queue-{id}"),
            title_id: None,
            episode_id: None,
            title_name: "Test Download".to_string(),
            facet: None,
            category: None,
            client_id: String::new(),
            client_name: String::new(),
            client_type: "mock".to_string(),
            state: scryer_domain::DownloadQueueState::Queued,
            progress_percent: 0,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: id.to_string(),
            download_id: None,
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

    #[tokio::test]
    async fn submit_download_skips_incompatible_clients_by_source_kind() {
        let torrent_client = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string(), "magnet_uri".to_string()],
                clients: vec![("torrent".to_string(), torrent_client.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("nzb", "NZBGet", "nzbget", 0),
                    test_config("torrent", "qBittorrent", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let result = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://tracker.example/file.torrent".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::TorrentFile),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("torrent request should route to torrent client");

        assert_eq!(result.client_type, "qbittorrent");
        assert_eq!(torrent_client.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_does_not_failover_ambiguous_submit_errors() {
        let primary = Arc::new(MockDownloadClient::default());
        *primary.submit_error.lock().unwrap() = Some(MockSubmitError::Ambiguous);
        let secondary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://tracker.example/file.torrent".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::TorrentFile),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("ambiguous submit errors should stop router failover");

        assert!(matches!(error, AppError::DownloadSubmitAmbiguous(_)));
        assert_eq!(primary.submissions.lock().unwrap().len(), 1);
        assert_eq!(secondary.submissions.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn submit_download_all_failover_clients_failed_returns_submit_unavailable() {
        let primary = Arc::new(MockDownloadClient::default());
        *primary.submit_error.lock().unwrap() = Some(MockSubmitError::Repository);
        let secondary = Arc::new(MockDownloadClient::default());
        *secondary.submit_error.lock().unwrap() = Some(MockSubmitError::SubmitUnavailable);
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://tracker.example/file.torrent".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::TorrentFile),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("exhausted failover clients should fail");

        assert!(matches!(error, AppError::DownloadSubmitUnavailable(_)));
        assert_eq!(primary.submissions.lock().unwrap().len(), 1);
        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_errors_when_no_enabled_client_can_handle_source_kind() {
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("nzb", "NZBGet", "nzbget", 0)],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            None,
        );

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("magnet:?xt=urn:btih:abcdef".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::MagnetUri),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("magnet request should fail when only nzb clients are enabled");

        match error {
            AppError::Validation(message) => {
                assert!(message.contains("magnet"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn submit_download_skips_clients_disabled_for_facet() {
        let primary = Arc::new(MockDownloadClient::default());
        let secondary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": { "enabled": false },
                        "secondary": { "enabled": true }
                    }"#
                    .to_string(),
                )]),
            }),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let result = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("secondary client should be used when primary is disabled for facet");

        assert_eq!(result.client_type, "qbittorrent");
        assert!(primary.submissions.lock().unwrap().is_empty());
        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_respects_facet_specific_enablement_per_scope() {
        let primary = Arc::new(MockDownloadClient::default());
        let secondary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([
                    (
                        "movie".to_string(),
                        r#"{
                            "primary": { "enabled": false },
                            "secondary": { "enabled": true }
                        }"#
                        .to_string(),
                    ),
                    (
                        "anime".to_string(),
                        r#"{
                            "primary": { "enabled": true },
                            "secondary": { "enabled": true }
                        }"#
                        .to_string(),
                    ),
                ]),
            }),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title_for_facet(MediaFacet::Movie),
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/movie.nzb".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Movie Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("movie request should use secondary");

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title_for_facet(MediaFacet::Anime),
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/anime.nzb".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Anime Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("anime request should use primary");

        assert_eq!(primary.submissions.lock().unwrap().len(), 1);
        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_ignores_facet_enabled_flag_for_globally_disabled_clients() {
        let secondary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("secondary".to_string(), secondary.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    disabled_test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": { "enabled": true },
                        "secondary": { "enabled": true }
                    }"#
                    .to_string(),
                )]),
            }),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("secondary client should be used because primary is globally disabled");

        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_applies_selected_client_category_and_recent_queue_priority() {
        let primary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("primary".to_string(), primary.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": {
                            "enabled": true,
                            "category": "Movies",
                            "recentQueuePriority": "high",
                            "olderQueuePriority": "low"
                        }
                    }"#
                    .to_string(),
                )]),
            }),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: Some("Fallback".to_string()),
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: Some(true),
                season_pack: None,
            })
            .await
            .expect("request should be routed");

        let submissions = primary.submissions.lock().unwrap();
        let request = submissions.first().expect("submission should be recorded");
        assert_eq!(request.category.as_deref(), Some("Movies"));
        assert_eq!(request.queue_priority.as_deref(), Some("high"));
    }

    #[tokio::test]
    async fn submit_download_uses_older_queue_priority_when_request_is_not_recent() {
        let primary = Arc::new(MockDownloadClient::default());
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("primary".to_string(), primary.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": {
                            "enabled": true,
                            "olderPriority": "very low"
                        }
                    }"#
                    .to_string(),
                )]),
            }),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: Some(false),
                season_pack: None,
            })
            .await
            .expect("request should be routed");

        let submissions = primary.submissions.lock().unwrap();
        let request = submissions.first().expect("submission should be recorded");
        assert_eq!(request.queue_priority.as_deref(), Some("very low"));
    }

    #[tokio::test]
    async fn submit_download_fails_when_all_clients_disabled_for_facet() {
        let fallback = Arc::new(MockDownloadClient::default());
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    "movie".to_string(),
                    r#"{
                        "primary": { "enabled": false }
                    }"#
                    .to_string(),
                )]),
            }),
            fallback.clone(),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![(
                    "primary".to_string(),
                    Arc::new(MockDownloadClient::default()),
                )],
            })),
        );

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title: test_title(),
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("facet-disabled clients should fail fast");

        match error {
            AppError::Validation(message) => {
                assert!(message.contains("no download client enabled"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }

        assert!(fallback.submissions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn submit_download_library_override_beats_facet_routing_for_eligibility() {
        let primary = Arc::new(MockDownloadClient::default());
        let secondary = Arc::new(MockDownloadClient::default());
        let title = test_title();
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([
                    (
                        "movie".to_string(),
                        r#"{
                            "primary": { "enabled": true },
                            "secondary": { "enabled": false }
                        }"#
                        .to_string(),
                    ),
                    (
                        title.library_id.clone(),
                        r#"{
                            "secondary": { "enabled": true }
                        }"#
                        .to_string(),
                    ),
                ]),
            }),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("library override should use the secondary client");

        assert!(primary.submissions.lock().unwrap().is_empty());
        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_treats_missing_library_override_clients_as_disabled() {
        let primary = Arc::new(MockDownloadClient::default());
        let secondary = Arc::new(MockDownloadClient::default());
        let title = test_title();
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("primary".to_string(), primary.clone()),
                    ("secondary".to_string(), secondary.clone()),
                ],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("primary", "Primary", "qbittorrent", 0),
                    test_config("secondary", "Secondary", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([
                    (
                        "movie".to_string(),
                        r#"{
                            "primary": { "enabled": true },
                            "secondary": { "enabled": true }
                        }"#
                        .to_string(),
                    ),
                    (
                        title.library_id.clone(),
                        r#"{
                            "secondary": { "enabled": true }
                        }"#
                        .to_string(),
                    ),
                ]),
            }),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect("omitted clients should be treated as disabled for this library");

        assert!(primary.submissions.lock().unwrap().is_empty());
        assert_eq!(secondary.submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submit_download_library_override_applies_category_and_queue_priority() {
        let primary = Arc::new(MockDownloadClient::default());
        let title = test_title();
        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("primary".to_string(), primary.clone())],
            });
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    title.library_id.clone(),
                    r#"{
                        "primary": {
                            "enabled": true,
                            "category": "Library Movies",
                            "recentQueuePriority": "high",
                            "olderQueuePriority": "low"
                        }
                    }"#
                    .to_string(),
                )]),
            }),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .submit_download(&DownloadClientAddRequest {
                title,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: Some("Fallback".to_string()),
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: Some(true),
                season_pack: None,
            })
            .await
            .expect("library override should route the request");

        let submissions = primary.submissions.lock().unwrap();
        let request = submissions.first().expect("submission should be recorded");
        assert_eq!(request.category.as_deref(), Some("Library Movies"));
        assert_eq!(request.queue_priority.as_deref(), Some("high"));
    }

    #[tokio::test]
    async fn submit_download_fails_when_all_clients_disabled_for_library_override() {
        let title = test_title();
        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("primary", "Primary", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository {
                routing_by_scope: HashMap::from([(
                    title.library_id.clone(),
                    r#"{
                        "primary": { "enabled": false }
                    }"#
                    .to_string(),
                )]),
            }),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![(
                    "primary".to_string(),
                    Arc::new(MockDownloadClient::default()),
                )],
            })),
        );

        let error = router
            .submit_download(&DownloadClientAddRequest {
                title,
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                download_id: None,
                source_hint: Some("https://example.invalid/release.nzb".to_string()),
                staged_nzb: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Test Release".to_string()),
                source_password: None,
                category: None,
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent: None,
                season_pack: None,
            })
            .await
            .expect_err("library override should fail fast when every client is disabled");

        match error {
            AppError::Validation(message) => {
                assert!(message.contains("no download client enabled for library"));
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pause_queue_item_routes_to_matching_client_item_id() {
        let nzb_client = Arc::new(MockDownloadClient::default());
        nzb_client
            .queue_items
            .lock()
            .unwrap()
            .push(test_queue_item("123"));

        let sab_client = Arc::new(MockDownloadClient::default());
        sab_client
            .queue_items
            .lock()
            .unwrap()
            .push(test_queue_item("SABnzbd_nzo_95u9pco9"));

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("nzb".to_string(), nzb_client.clone()),
                    ("sab".to_string(), sab_client.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("nzb", "NZBGet", "nzbget", 0),
                    test_config("sab", "SABnzbd", "sabnzbd", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .pause_queue_item("SABnzbd_nzo_95u9pco9")
            .await
            .expect("pause should route to sabnzbd client");

        assert!(nzb_client.paused.lock().unwrap().is_empty());
        assert_eq!(
            sab_client.paused.lock().unwrap().as_slice(),
            ["SABnzbd_nzo_95u9pco9"]
        );
    }

    #[tokio::test]
    async fn delete_history_item_routes_to_matching_client_item_id() {
        let nzb_client = Arc::new(MockDownloadClient::default());
        nzb_client
            .history_items
            .lock()
            .unwrap()
            .push(test_queue_item("42"));

        let sab_client = Arc::new(MockDownloadClient::default());
        sab_client
            .history_items
            .lock()
            .unwrap()
            .push(test_queue_item("SABnzbd_nzo_hist01"));

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("nzb".to_string(), nzb_client.clone()),
                    ("sab".to_string(), sab_client.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("nzb", "NZBGet", "nzbget", 0),
                    test_config("sab", "SABnzbd", "sabnzbd", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        router
            .delete_queue_item("SABnzbd_nzo_hist01", true)
            .await
            .expect("history delete should route to sabnzbd client");

        assert!(nzb_client.deleted.lock().unwrap().is_empty());
        assert_eq!(
            sab_client.deleted.lock().unwrap().as_slice(),
            [("SABnzbd_nzo_hist01".to_string(), true)]
        );
    }

    #[tokio::test]
    async fn list_history_page_merges_clients_before_slicing() {
        let client_a = Arc::new(MockDownloadClient::default());
        let client_b = Arc::new(MockDownloadClient::default());

        let mut a1 = test_queue_item("a-1");
        a1.last_updated_at = Some("300".to_string());
        let mut a2 = test_queue_item("a-2");
        a2.last_updated_at = Some("100".to_string());
        client_a.history_items.lock().unwrap().extend([a1, a2]);

        let mut b1 = test_queue_item("b-1");
        b1.last_updated_at = Some("200".to_string());
        let mut b2 = test_queue_item("b-2");
        b2.last_updated_at = Some("50".to_string());
        client_b.history_items.lock().unwrap().extend([b1, b2]);

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("client-a".to_string(), client_a.clone()),
                    ("client-b".to_string(), client_b.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("client-a", "Client A", "qbittorrent", 0),
                    test_config("client-b", "Client B", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let page = router
            .list_history_page(1, 2)
            .await
            .expect("paged history should succeed");

        let ids = page
            .into_iter()
            .map(|item| item.download_client_item_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["b-1".to_string(), "a-2".to_string()]);
    }

    #[tokio::test]
    async fn list_recent_activity_merges_clients_before_truncating() {
        let client_a = Arc::new(MockDownloadClient::default());
        let client_b = Arc::new(MockDownloadClient::default());

        let mut a1 = test_queue_item("a-1");
        a1.last_updated_at = Some("300".to_string());
        let mut a2 = test_queue_item("a-2");
        a2.last_updated_at = Some("100".to_string());
        client_a.history_items.lock().unwrap().extend([a1, a2]);

        let mut b1 = test_queue_item("b-1");
        b1.last_updated_at = Some("200".to_string());
        let mut b2 = test_queue_item("b-2");
        b2.last_updated_at = Some("50".to_string());
        client_b.history_items.lock().unwrap().extend([b1, b2]);

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("client-a".to_string(), client_a.clone()),
                    ("client-b".to_string(), client_b.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("client-a", "Client A", "qbittorrent", 0),
                    test_config("client-b", "Client B", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let items = router
            .list_recent_activity(2)
            .await
            .expect("recent activity should succeed");

        let ids = items
            .into_iter()
            .map(|item| item.download_client_item_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["a-1".to_string(), "b-1".to_string()]);
    }

    #[tokio::test]
    async fn list_recent_completed_downloads_merges_clients_before_truncating() {
        let client_a = Arc::new(MockDownloadClient::default());
        let client_b = Arc::new(MockDownloadClient::default());

        client_a.completed_downloads.lock().unwrap().extend([
            scryer_domain::CompletedDownload {
                client_type: "qbittorrent".to_string(),
                client_id: String::new(),
                download_client_item_id: "a-1".to_string(),
                download_id: None,
                name: "A 1".to_string(),
                dest_dir: "/downloads/a-1".to_string(),
                category: None,
                size_bytes: None,
                completed_at: Some(Utc::now()),
                parameters: Vec::new(),
            },
            scryer_domain::CompletedDownload {
                client_type: "qbittorrent".to_string(),
                client_id: String::new(),
                download_client_item_id: "a-2".to_string(),
                download_id: None,
                name: "A 2".to_string(),
                dest_dir: "/downloads/a-2".to_string(),
                category: None,
                size_bytes: None,
                completed_at: Some(Utc::now() - chrono::Duration::minutes(2)),
                parameters: Vec::new(),
            },
        ]);
        client_b.completed_downloads.lock().unwrap().extend([
            scryer_domain::CompletedDownload {
                client_type: "qbittorrent".to_string(),
                client_id: String::new(),
                download_client_item_id: "b-1".to_string(),
                download_id: None,
                name: "B 1".to_string(),
                dest_dir: "/downloads/b-1".to_string(),
                category: None,
                size_bytes: None,
                completed_at: Some(Utc::now() - chrono::Duration::minutes(1)),
                parameters: Vec::new(),
            },
            scryer_domain::CompletedDownload {
                client_type: "qbittorrent".to_string(),
                client_id: String::new(),
                download_client_item_id: "b-2".to_string(),
                download_id: None,
                name: "B 2".to_string(),
                dest_dir: "/downloads/b-2".to_string(),
                category: None,
                size_bytes: None,
                completed_at: Some(Utc::now() - chrono::Duration::minutes(3)),
                parameters: Vec::new(),
            },
        ]);

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("client-a".to_string(), client_a.clone()),
                    ("client-b".to_string(), client_b.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("client-a", "Client A", "qbittorrent", 0),
                    test_config("client-b", "Client B", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let items = router
            .list_recent_completed_downloads(2)
            .await
            .expect("recent completed downloads should succeed");

        let ids = items
            .into_iter()
            .map(|item| item.download_client_item_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["a-1".to_string(), "b-1".to_string()]);
    }

    #[tokio::test]
    async fn list_completed_downloads_applies_remote_path_mappings_from_client_config() {
        let client = Arc::new(MockDownloadClient::default());
        client
            .completed_downloads
            .lock()
            .unwrap()
            .push(scryer_domain::CompletedDownload {
                client_type: "qbittorrent".to_string(),
                client_id: String::new(),
                download_client_item_id: "remote-1".to_string(),
                download_id: None,
                name: "Remote Download".to_string(),
                dest_dir: "D:\\Data\\Completed\\Remote Download".to_string(),
                category: None,
                size_bytes: None,
                completed_at: Some(Utc::now()),
                parameters: Vec::new(),
            });

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string()],
                clients: vec![("client-a".to_string(), client.clone())],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![DownloadClientConfig {
                    config_json:
                        r#"{"remote_path_mappings":"D:\\Data\\Completed => /Volumes/downloads"}"#
                            .to_string(),
                    ..test_config("client-a", "Client A", "qbittorrent", 0)
                }],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let items = router
            .list_completed_downloads()
            .await
            .expect("completed downloads should succeed");

        assert_eq!(items[0].client_id, "client-a");
        assert_eq!(items[0].dest_dir, "/Volumes/downloads/Remote Download");
    }

    #[tokio::test]
    async fn get_client_status_for_client_id_applies_remote_path_mappings_to_output_roots() {
        let client = Arc::new(MockDownloadClient::default());
        *client.status.lock().unwrap() = DownloadClientStatus {
            is_localhost: Some(false),
            remote_output_roots: vec!["/downloads/complete".to_string()],
            ..DownloadClientStatus::default()
        };

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["torrent_file".to_string()],
                clients: vec![("client-a".to_string(), client.clone())],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![DownloadClientConfig {
                    config_json: r#"{"remote_path_mappings":"/downloads => /Volumes/downloads"}"#
                        .to_string(),
                    ..test_config("client-a", "Client A", "qbittorrent", 0)
                }],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let status = router
            .get_client_status_for_client_id("client-a")
            .await
            .expect("client status should succeed");

        assert_eq!(
            status.remote_output_roots,
            vec!["/Volumes/downloads/complete".to_string()]
        );
    }

    #[tokio::test]
    async fn download_client_timeout_wrapper_times_out_feedback_reads() {
        let wrapped = FeedbackTimeoutDownloadClient::new(
            Arc::new(DelayedQueueDownloadClient {
                delay: Duration::from_millis(25),
                queue_items: vec![test_queue_item("slow")],
            }),
            Duration::from_millis(5),
        );

        let error = wrapped
            .list_queue()
            .await
            .expect_err("slow feedback reads should time out");

        assert!(matches!(
            error,
            AppError::DownloadFeedbackTimeout(ref message)
                if message == DOWNLOAD_FEEDBACK_TIMEOUT_MESSAGE
        ));
    }

    #[tokio::test]
    async fn list_queue_returns_partial_data_when_a_client_times_out() {
        let fast_client = Arc::new(MockDownloadClient::default());
        fast_client
            .queue_items
            .lock()
            .unwrap()
            .push(test_queue_item("fast"));

        let slow_client: Arc<dyn DownloadClient> = Arc::new(DelayedQueueDownloadClient {
            delay: Duration::from_millis(25),
            queue_items: vec![test_queue_item("slow")],
        });

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("fast".to_string(), fast_client.clone()),
                    ("slow".to_string(), slow_client),
                ],
            });

        let router = PrioritizedDownloadClientRouter::with_feedback_read_timeout(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("fast", "Fast", "qbittorrent", 0),
                    test_config("slow", "Slow", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
            Duration::from_millis(5),
        );

        let items = router
            .list_queue()
            .await
            .expect("partial data should still succeed when one client times out");

        let ids = items
            .into_iter()
            .map(|item| item.download_client_item_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["fast".to_string()]);
    }

    #[tokio::test]
    async fn list_queue_returns_timeout_error_when_all_clients_time_out() {
        let slow_a: Arc<dyn DownloadClient> = Arc::new(DelayedQueueDownloadClient {
            delay: Duration::from_millis(25),
            queue_items: vec![test_queue_item("slow-a")],
        });
        let slow_b: Arc<dyn DownloadClient> = Arc::new(DelayedQueueDownloadClient {
            delay: Duration::from_millis(25),
            queue_items: vec![test_queue_item("slow-b")],
        });

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("slow-a".to_string(), slow_a),
                    ("slow-b".to_string(), slow_b),
                ],
            });

        let router = PrioritizedDownloadClientRouter::with_feedback_read_timeout(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("slow-a", "Slow A", "qbittorrent", 0),
                    test_config("slow-b", "Slow B", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
            Duration::from_millis(5),
        );

        let error = router
            .list_queue()
            .await
            .expect_err("timeout-only outages should surface as typed timeout errors");

        assert!(matches!(
            error,
            AppError::DownloadFeedbackTimeout(ref message)
                if message == DOWNLOAD_FEEDBACK_TIMEOUT_MESSAGE
        ));
    }

    #[tokio::test]
    async fn list_queue_backs_off_after_feedback_failures() {
        let fast_client = Arc::new(MockDownloadClient::default());
        fast_client
            .queue_items
            .lock()
            .unwrap()
            .push(test_queue_item("fast"));
        let failing_client = Arc::new(FailingQueueDownloadClient::default());

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![
                    ("fast".to_string(), fast_client.clone()),
                    ("failing".to_string(), failing_client.clone()),
                ],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![
                    test_config("fast", "Fast", "qbittorrent", 0),
                    test_config("failing", "Failing", "qbittorrent", 1),
                ],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let first = router
            .list_queue()
            .await
            .expect("first queue read should succeed");
        let second = router
            .list_queue()
            .await
            .expect("backed off queue read should succeed");

        assert_eq!(failing_client.list_queue_call_count(), 1);
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].download_client_item_id, "fast");
        assert_eq!(second[0].download_client_item_id, "fast");
    }

    #[tokio::test]
    async fn list_queue_for_title_bypasses_feedback_backoff() {
        let failing_client = Arc::new(FailingQueueDownloadClient::default());

        let plugin_provider: Arc<dyn DownloadClientPluginProvider> =
            Arc::new(MockDownloadClientPluginProvider {
                accepted_inputs: vec!["nzb_url".to_string()],
                clients: vec![("failing".to_string(), failing_client.clone())],
            });

        let router = PrioritizedDownloadClientRouter::new(
            Arc::new(MockDownloadClientConfigRepository {
                configs: vec![test_config("failing", "Failing", "qbittorrent", 0)],
            }),
            Arc::new(MockSettingsRepository::default()),
            Arc::new(MockDownloadClient::default()),
            null_staged_nzb_store(),
            test_pipeline_limit(),
            Some(plugin_provider),
        );

        let _ = router
            .list_queue()
            .await
            .expect("queue read should degrade to empty");
        let _ = router
            .list_queue_for_title("title-1")
            .await
            .expect("title-scoped queue read should bypass backoff");

        assert_eq!(failing_client.list_queue_call_count(), 1);
        assert_eq!(failing_client.list_queue_for_title_call_count(), 1);
    }
}
