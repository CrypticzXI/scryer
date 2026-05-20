//! TrackedDownloads — scryer-side download lifecycle state machine (plan 055).
//!
//! Maintains an in-memory cache of active downloads, each enriched with title
//! resolution metadata and driven through a workflow state machine independent
//! of the download client's reported status.

use chrono::{DateTime, Utc};
use scryer_domain::{
    DownloadQueueItem, Title, TitleMatchType, TrackedDownloadState, TrackedDownloadStatus,
};
use std::collections::{HashMap, HashSet};
use tokio::sync::{mpsc, oneshot};

use crate::{AppResult, AppUseCase, DownloadSourceIdentity, DownloadSubmission, SubmissionScope};

const DEFAULT_TRACKED_DOWNLOAD_CACHE_TTL_HOURS: i64 = 24;
const DEFAULT_TRACKED_DOWNLOAD_CACHE_MAX_ENTRIES: usize = 5_000;

// ── TrackedDownload ──────────────────────────────────────────────────────────

/// A download being tracked through scryer's import workflow.
#[derive(Clone, Debug)]
pub struct TrackedDownload {
    /// Composite key scoped to the configured client when available.
    pub id: String,
    pub client_id: String,
    pub client_type: String,
    /// Latest snapshot from the download client.
    pub client_item: DownloadQueueItem,
    /// Scryer's workflow state (independent of client status).
    pub state: TrackedDownloadState,
    /// Health/warning overlay.
    pub status: TrackedDownloadStatus,
    /// Human-readable status messages.
    pub status_messages: Vec<String>,
    /// Resolved scryer title.
    pub title_id: Option<String>,
    pub facet: Option<String>,
    /// Release name from grab history (fallback parsing source).
    pub source_title: Option<String>,
    pub indexer: Option<String>,
    pub added_at: Option<DateTime<Utc>>,
    /// Whether the user has been notified about manual intervention.
    pub notified_manual_interaction: bool,
    /// How the title was resolved.
    pub match_type: TitleMatchType,
    /// Whether this download is still visible in the client.
    pub is_trackable: bool,
    /// Whether import() has been called at least once. Prevents check() from
    /// re-evaluating a post-import ImportBlocked back to ImportPending.
    pub import_attempted: bool,
    /// When a completed download path first became unavailable.
    pub path_missing_since: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackedDownloadQueueMetadata {
    pub client_item: DownloadQueueItem,
    pub title_id: Option<String>,
    pub facet: Option<String>,
    pub source_title: Option<String>,
    pub state: TrackedDownloadState,
    pub status: TrackedDownloadStatus,
    pub status_messages: Vec<String>,
    pub match_type: TitleMatchType,
}

impl From<&TrackedDownload> for TrackedDownloadQueueMetadata {
    fn from(value: &TrackedDownload) -> Self {
        Self {
            client_item: value.client_item.clone(),
            title_id: value.title_id.clone(),
            facet: value.facet.clone(),
            source_title: value.source_title.clone(),
            state: value.state,
            status: value.status,
            status_messages: value.status_messages.clone(),
            match_type: value.match_type,
        }
    }
}

impl TrackedDownload {
    pub fn warn(&mut self, message: impl Into<String>) {
        self.status = TrackedDownloadStatus::Warning;
        self.status_messages.push(message.into());
    }

    pub fn clear_warnings(&mut self) {
        self.status = TrackedDownloadStatus::Ok;
        self.status_messages.clear();
    }

    pub fn fail(&mut self) {
        self.status = TrackedDownloadStatus::Error;
        self.state = TrackedDownloadState::FailedPending;
    }
}

// ── TrackedDownloadService ───────────────────────────────────────────────────

/// In-memory cache of tracked downloads with title resolution and state management.
#[derive(Default)]
pub struct TrackedDownloadService {
    cache: HashMap<String, TrackedDownload>,
    last_seen_at: HashMap<String, DateTime<Utc>>,
}

impl TrackedDownloadService {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create or update a tracked download from a client item snapshot.
    ///
    /// On first see: resolves title, checks for terminal state in DB.
    /// On update: refreshes client_item but preserves scryer state if past Downloading.
    pub async fn track(&mut self, app: &AppUseCase, client_item: DownloadQueueItem) {
        let id = tracked_download_id(
            Some(client_item.client_id.as_str()),
            &client_item.client_type,
            &client_item.download_client_item_id,
        );
        self.last_seen_at.insert(id.clone(), Utc::now());

        if self.cache.contains_key(&id) {
            let existing = self.cache.get_mut(&id).unwrap();
            let matcher_dirty = app
                .runtime
                .catalog
                .monitored_title_matcher
                .read()
                .await
                .dirty;
            let should_reresolve = should_reresolve_title(existing, &client_item, matcher_dirty);
            // Update the client snapshot but preserve scryer state if not Downloading.
            if existing.state == TrackedDownloadState::Downloading {
                existing.status = TrackedDownloadStatus::Ok;
                existing.status_messages.clear();
            }
            existing.client_item = client_item;
            existing.is_trackable = true;
            if should_reresolve {
                Self::resolve_title(app, existing).await;
            }
            return;
        }

        // First time seeing this download — build, resolve, and insert.
        let td = Self::build_new_tracked_download(app, id.clone(), client_item).await;
        self.cache.insert(id, td);
        self.prune_cache();
    }

    /// Build a new TrackedDownload, resolving title and reconstructing state.
    async fn build_new_tracked_download(
        app: &AppUseCase,
        id: String,
        client_item: DownloadQueueItem,
    ) -> TrackedDownload {
        let mut td = TrackedDownload {
            id,
            client_id: client_item.client_id.clone(),
            client_type: client_item.client_type.clone(),
            title_id: client_item.title_id.clone(),
            facet: client_item.facet.clone(),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Unmatched,
            is_trackable: true,
            state: TrackedDownloadState::Downloading,
            status: TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            client_item,
            import_attempted: false,
            path_missing_since: None,
        };

        Self::resolve_title(app, &mut td).await;
        Self::reconstruct_state(app, &mut td).await;
        td
    }

    pub fn find(&self, id: &str) -> Option<&TrackedDownload> {
        self.cache.get(id)
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut TrackedDownload> {
        self.cache.get_mut(id)
    }

    pub fn get_all(&self) -> Vec<&TrackedDownload> {
        self.cache.values().collect()
    }

    pub fn get_trackable(&self) -> Vec<&TrackedDownload> {
        self.cache
            .values()
            .filter(|td| td.is_trackable && !td.state.is_terminal())
            .collect()
    }

    pub fn get_trackable_ids(&self) -> Vec<String> {
        self.cache
            .values()
            .filter(|td| td.is_trackable && !td.state.is_terminal())
            .map(|td| td.id.clone())
            .collect()
    }

    /// Mark downloads no longer visible in any client as untrackable.
    pub fn update_trackable(&mut self, seen_ids: &HashSet<String>) {
        for td in self.cache.values_mut() {
            let should_preserve_tracking = matches!(
                td.state,
                TrackedDownloadState::ImportPending
                    | TrackedDownloadState::Importing
                    | TrackedDownloadState::FailedPending
            );
            if !seen_ids.contains(&td.id) && !should_preserve_tracking {
                td.is_trackable = false;
            }
        }
        self.prune_cache();
    }

    /// Remove a download from the cache (after terminal state).
    pub fn stop_tracking(&mut self, id: &str) {
        self.cache.remove(id);
        self.last_seen_at.remove(id);
    }

    fn prune_cache(&mut self) {
        let ttl = tracked_download_cache_ttl();
        let stale_cutoff = Utc::now() - ttl;
        let last_seen_at = &self.last_seen_at;
        self.cache.retain(|id, tracked| {
            tracked.is_trackable
                || last_seen_at
                    .get(id)
                    .is_none_or(|last_seen| *last_seen >= stale_cutoff)
        });

        let max_entries = tracked_download_cache_max_entries();
        if self.cache.len() > max_entries {
            let mut eviction_candidates = self
                .cache
                .iter()
                .filter(|(_, tracked)| !tracked.is_trackable)
                .map(|(id, _)| {
                    (
                        self.last_seen_at.get(id).copied().unwrap_or(stale_cutoff),
                        id.clone(),
                    )
                })
                .collect::<Vec<_>>();
            eviction_candidates.sort_by_key(|(last_seen, _)| *last_seen);
            let overage = self.cache.len().saturating_sub(max_entries);
            for (_, id) in eviction_candidates.into_iter().take(overage) {
                self.cache.remove(&id);
            }
        }

        self.last_seen_at
            .retain(|id, _| self.cache.contains_key(id));
    }

    /// Persist a terminal state to download_submissions.
    pub async fn persist_terminal_state(
        &self,
        app: &AppUseCase,
        id: &str,
        state: TrackedDownloadState,
    ) -> bool {
        if !state.is_terminal() {
            return true;
        }
        let Some(td) = self.cache.get(id) else {
            return false;
        };
        if let Err(e) = app
            .services
            .workflow
            .download_submissions
            .update_tracked_state(
                &DownloadSourceIdentity::new(
                    Some(td.client_id.as_str()),
                    &td.client_type,
                    &td.client_item.download_client_item_id,
                ),
                state.as_str(),
            )
            .await
        {
            tracing::warn!(
                error = %e,
                id,
                state = state.as_str(),
                "failed to persist tracked download terminal state"
            );
            return false;
        }

        true
    }

    // ── Title Resolution ─────────────────────────────────────────────────

    async fn resolve_title(app: &AppUseCase, td: &mut TrackedDownload) {
        let existing_submission = app
            .services
            .workflow
            .download_submissions
            .find_by_client_item_id(&DownloadSourceIdentity::new(
                Some(td.client_id.as_str()),
                &td.client_type,
                &td.client_item.download_client_item_id,
            ))
            .await
            .ok()
            .flatten();

        // 1. download_submissions lookup (highest confidence).
        if let Some(sub) = existing_submission.as_ref()
            && !sub.title_id.is_empty()
        {
            td.title_id = Some(sub.title_id.clone());
            td.facet = Some(sub.facet.clone());
            td.source_title = sub.source_title.clone();
            td.match_type = TitleMatchType::Submission;
            return;
        }

        // 2. Embedded client parameters (*scryer_title_id).
        if let Some(title_id) = td.client_item.title_id.as_deref().filter(|s| !s.is_empty()) {
            // Cross-validate: does this title still exist?
            if let Ok(Some(_)) = app.services.catalog.titles.get_by_id(title_id).await {
                td.title_id = Some(title_id.to_string());
                td.match_type = TitleMatchType::ClientParameter;
                return;
            }
        }

        // 3. Parse-based monitored title resolution for foreign downloads.
        let release_title = td
            .source_title
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(td.client_item.title_name.as_str());
        let parsed = crate::parse_release_metadata(release_title);
        if let Ok(matcher) = app.monitored_title_matcher().await {
            let matched = if parsed.episode.is_some() {
                matcher.resolve_episode(
                    &parsed,
                    td.client_item.facet.as_deref().or(td.facet.as_deref()),
                )
            } else {
                matcher.resolve_movie(&parsed)
            };

            if let Some(resolved) = matched {
                td.title_id = Some(resolved.title.id.clone());
                td.facet = Some(resolved.title.facet.as_str().to_string());
                if td.source_title.is_none() {
                    td.source_title = Some(release_title.to_string());
                }
                td.match_type = resolved.match_type;
                return;
            }
        }

        // 4. No trustworthy title match found — completed handler will block
        // auto-import until the user assigns the title manually.
        //
        // Insert a stub download_submissions row for foreign downloads so they
        // get a tracked_state column for restart reconstruction.
        if existing_submission.is_none()
            && let Err(error) = app
                .services
                .workflow
                .download_submissions
                .record_submission(DownloadSubmission {
                    title_id: String::new(),
                    facet: td.facet.clone().unwrap_or_default(),
                    download_client_id: Some(td.client_id.clone())
                        .filter(|value| !value.is_empty()),
                    download_client_type: td.client_type.clone(),
                    download_client_item_id: td.client_item.download_client_item_id.clone(),
                    source_hint: None,
                    source_kind: None,
                    source_title: Some(td.client_item.title_name.clone()),
                    request_signature: None,
                    scope: SubmissionScope::Orphan,
                })
                .await
        {
            tracing::warn!(error = %error, id = %td.id, "failed to record tracked download stub submission");
        }
    }

    /// Reconstruct state from persistent storage after restart.
    async fn reconstruct_state(app: &AppUseCase, td: &mut TrackedDownload) {
        // Check download_submissions.tracked_state for terminal states.
        if let Ok(Some(tracked_state)) = app
            .services
            .workflow
            .download_submissions
            .get_tracked_state(&DownloadSourceIdentity::new(
                Some(td.client_id.as_str()),
                &td.client_type,
                &td.client_item.download_client_item_id,
            ))
            .await
            && let Some(state) = TrackedDownloadState::from_str_opt(&tracked_state)
            && state.is_terminal()
        {
            td.state = state;
            return;
        }

        // Fall back to the latest import record for restart recovery if the
        // tracked state was not persisted before shutdown.
        if let Ok(true) = app
            .services
            .workflow
            .imports
            .is_already_imported(&DownloadSourceIdentity::new(
                Some(td.client_id.as_str()),
                &td.client_type,
                &td.client_item.download_client_item_id,
            ))
            .await
        {
            td.state = TrackedDownloadState::Imported;
            let _ = app
                .services
                .workflow
                .download_submissions
                .update_tracked_state(
                    &DownloadSourceIdentity::new(
                        Some(td.client_id.as_str()),
                        &td.client_type,
                        &td.client_item.download_client_item_id,
                    ),
                    TrackedDownloadState::Imported.as_str(),
                )
                .await;
        }

        // Default: Downloading (will be re-evaluated by check cycle).
    }
}

pub(crate) async fn publish_runtime_tracked_download_snapshot(
    app: &AppUseCase,
    tracked: &TrackedDownload,
) {
    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked.id.clone(),
            TrackedDownloadQueueMetadata::from(tracked),
        );
}

pub(crate) async fn publish_runtime_tracked_download_snapshot_cache(
    app: &AppUseCase,
    tracker: &TrackedDownloadService,
) {
    let snapshot = tracker
        .get_all()
        .into_iter()
        .filter(|tracked| tracked.is_trackable)
        .map(|tracked| {
            (
                tracked.id.clone(),
                TrackedDownloadQueueMetadata::from(tracked),
            )
        })
        .collect::<HashMap<_, _>>();
    *app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await = snapshot;
}

fn title_id_present(value: Option<&str>) -> bool {
    value.is_some_and(|id| !id.trim().is_empty())
}

fn normalize_title_signal(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut last_was_space = false;
    for ch in value.chars() {
        if ch.is_alphanumeric() {
            normalized.extend(ch.to_lowercase());
            last_was_space = false;
        } else if !last_was_space {
            normalized.push(' ');
            last_was_space = true;
        }
    }
    normalized.trim().to_string()
}

fn normalize_facet_signal(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn should_reresolve_title(
    existing: &TrackedDownload,
    incoming: &DownloadQueueItem,
    matcher_dirty: bool,
) -> bool {
    if matches!(
        existing.match_type,
        TitleMatchType::Submission | TitleMatchType::ClientParameter
    ) {
        return false;
    }

    if matcher_dirty
        && matches!(
            existing.match_type,
            TitleMatchType::Unmatched | TitleMatchType::IdOnly | TitleMatchType::TitleParse
        )
    {
        return true;
    }

    if !title_id_present(existing.client_item.title_id.as_deref())
        && title_id_present(incoming.title_id.as_deref())
    {
        return true;
    }

    if !title_id_present(existing.title_id.as_deref())
        && title_id_present(incoming.title_id.as_deref())
    {
        return true;
    }

    if !existing.client_item.is_scryer_origin && incoming.is_scryer_origin {
        return true;
    }

    if normalize_title_signal(&existing.client_item.title_name)
        != normalize_title_signal(&incoming.title_name)
    {
        return true;
    }

    if normalize_facet_signal(existing.client_item.facet.as_deref())
        != normalize_facet_signal(incoming.facet.as_deref())
    {
        return true;
    }

    false
}

pub(crate) async fn assign_title_to_tracked_download(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    title: &Title,
) {
    td.title_id = Some(title.id.clone());
    td.facet = Some(title.facet.as_str().to_string());
    td.match_type = TitleMatchType::Submission;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages.clear();
    td.import_attempted = false;

    // A download that is already blocked for manual intervention should stay
    // manually actionable after title assignment instead of being pushed
    // straight back into auto-import.
    if td.state == TrackedDownloadState::ImportBlocked {
        return;
    }

    td.state = TrackedDownloadState::Downloading;
    crate::failed_download_handler::check(td);
    crate::completed_download_handler::check(app, td).await;
}

// ── Command Channel ──────────────────────────────────────────────────────────

/// Commands sent from GraphQL mutations to the poller's TrackedDownloadService.
pub enum TrackedDownloadCommand {
    MarkImported {
        id: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
    Ignore {
        id: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
    MarkFailed {
        id: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
    RetryImport {
        id: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
    AssignTitle {
        id: String,
        title_id: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
    Snapshot {
        ids: Vec<String>,
        reply: oneshot::Sender<HashMap<String, TrackedDownloadQueueMetadata>>,
    },
}

/// Handle for sending commands to the tracked downloads poller.
#[derive(Clone)]
pub struct TrackedDownloadHandle {
    tx: mpsc::Sender<TrackedDownloadCommand>,
}

impl TrackedDownloadHandle {
    pub fn new(tx: mpsc::Sender<TrackedDownloadCommand>) -> Self {
        Self { tx }
    }

    pub async fn ignore(&self, id: String) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::Ignore {
                id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn mark_imported(&self, id: String) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::MarkImported {
                id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn mark_failed(&self, id: String) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::MarkFailed {
                id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn retry_import(&self, id: String) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::RetryImport {
                id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn assign_title(&self, id: String, title_id: String) -> AppResult<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::AssignTitle {
                id,
                title_id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })?
    }

    pub async fn snapshot(
        &self,
        ids: Vec<String>,
    ) -> AppResult<HashMap<String, TrackedDownloadQueueMetadata>> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(TrackedDownloadCommand::Snapshot {
                ids,
                reply: reply_tx,
            })
            .await
            .map_err(|_| {
                crate::AppError::Repository("tracked download service unavailable".into())
            })?;
        reply_rx.await.map_err(|_| {
            crate::AppError::Repository("tracked download service dropped reply".into())
        })
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn tracked_download_cache_ttl() -> chrono::Duration {
    std::env::var("SCRYER_TRACKED_DOWNLOAD_CACHE_TTL_HOURS")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|hours| *hours > 0)
        .map(chrono::Duration::hours)
        .unwrap_or_else(|| chrono::Duration::hours(DEFAULT_TRACKED_DOWNLOAD_CACHE_TTL_HOURS))
}

fn tracked_download_cache_max_entries() -> usize {
    std::env::var("SCRYER_TRACKED_DOWNLOAD_CACHE_MAX_ENTRIES")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|entries| *entries > 0)
        .unwrap_or(DEFAULT_TRACKED_DOWNLOAD_CACHE_MAX_ENTRIES)
}

pub fn tracked_download_id(client_id: Option<&str>, client_type: &str, item_id: &str) -> String {
    let normalized_client_id = client_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    if normalized_client_id.is_empty() {
        return format!("{client_type}:{item_id}");
    }

    format!("{normalized_client_id}:{item_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
        NullQualityProfileRepository, NullReleaseAttemptRepository, NullShowRepository,
        NullTitleRepository, NullUserRepository,
    };
    use crate::{
        AppError, AppResult, AppServices, AppUseCase, CreateTitleOutcome, DomainEventRepository,
        DownloadClient, DownloadClientAddRequest, DownloadGrabResult, DownloadSourceIdentity,
        DownloadSubmissionRepository, FacetRegistry, ImportRepository, IndexerConfigRepository,
        JwtAuthConfig, PendingTitleHydration, TitleMetadataUpdate, TitleRepository,
    };
    use async_trait::async_trait;
    use chrono::Utc;
    use scryer_domain::{
        CompletedDownload, DomainEvent, DomainEventFilter, DownloadQueueState, Id, ImportRecord,
        ImportStatus, ImportType, MediaFacet, NewDomainEvent, Title, TitleHistoryEventType, User,
    };
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct TestDownloadSubmissionRepo {
        submission: Option<crate::DownloadSubmission>,
        tracked_state: Option<String>,
        tracked_state_updates: Arc<Mutex<Vec<String>>>,
        recorded_submissions: Arc<Mutex<Vec<crate::DownloadSubmission>>>,
    }

    #[async_trait]
    impl DownloadSubmissionRepository for TestDownloadSubmissionRepo {
        async fn record_submission(&self, submission: crate::DownloadSubmission) -> AppResult<()> {
            self.recorded_submissions.lock().await.push(submission);
            Ok(())
        }

        async fn find_by_client_item_id(
            &self,
            _: &DownloadSourceIdentity,
        ) -> AppResult<Option<crate::DownloadSubmission>> {
            Ok(self.submission.clone())
        }

        async fn list_for_client_items(
            &self,
            _: &[DownloadSourceIdentity],
        ) -> AppResult<Vec<crate::DownloadSubmission>> {
            Ok(self.submission.clone().into_iter().collect())
        }

        async fn list_for_title(&self, _: &str) -> AppResult<Vec<crate::DownloadSubmission>> {
            Ok(vec![])
        }

        async fn find_by_title_and_request_signature(
            &self,
            _: &str,
            _: &str,
        ) -> AppResult<Option<crate::DownloadSubmission>> {
            Ok(None)
        }

        async fn delete_for_title(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn delete_by_client_item_id(&self, _: &DownloadSourceIdentity) -> AppResult<()> {
            Ok(())
        }

        async fn update_tracked_state(
            &self,
            _: &DownloadSourceIdentity,
            tracked_state: &str,
        ) -> AppResult<()> {
            self.tracked_state_updates
                .lock()
                .await
                .push(tracked_state.to_string());
            Ok(())
        }

        async fn get_tracked_state(&self, _: &DownloadSourceIdentity) -> AppResult<Option<String>> {
            Ok(self.tracked_state.clone())
        }
    }

    struct TestImportStatusUpdate(ImportStatus, Option<String>);

    #[derive(Default)]
    struct TestImportRepo {
        import_record: Option<ImportRecord>,
        import_records: Vec<ImportRecord>,
        status_updates: Arc<Mutex<Vec<TestImportStatusUpdate>>>,
    }

    impl TestImportRepo {
        fn stored_imports(&self) -> Vec<ImportRecord> {
            if !self.import_records.is_empty() {
                return self.import_records.clone();
            }

            self.import_record.clone().into_iter().collect()
        }
    }

    #[derive(Default)]
    struct TestDownloadClient {
        queue_items: Arc<Mutex<Vec<DownloadQueueItem>>>,
        recent_activity: Arc<Mutex<Vec<DownloadQueueItem>>>,
        completed_downloads: Arc<Mutex<Vec<CompletedDownload>>>,
    }

    #[async_trait]
    impl DownloadClient for TestDownloadClient {
        async fn submit_download(
            &self,
            _: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
            Ok(self.completed_downloads.lock().await.clone())
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            Ok(self.queue_items.lock().await.clone())
        }

        async fn list_recent_activity(&self, _: usize) -> AppResult<Vec<DownloadQueueItem>> {
            Ok(self.recent_activity.lock().await.clone())
        }
    }

    #[derive(Default)]
    struct TestDomainEventRepo {
        events: Arc<Mutex<Vec<DomainEvent>>>,
        subscriber_offsets: Arc<Mutex<HashMap<String, i64>>>,
    }

    #[derive(Default)]
    struct TestTitleRepo {
        titles: Vec<Title>,
    }

    struct MutableTitleRepo {
        titles: Arc<Mutex<Vec<Title>>>,
        list_for_matching_calls: Arc<Mutex<usize>>,
    }

    #[derive(Default)]
    struct TestIndexerConfigRepo;

    #[async_trait]
    impl IndexerConfigRepository for TestIndexerConfigRepo {
        async fn list(&self, _: Option<String>) -> AppResult<Vec<scryer_domain::IndexerConfig>> {
            Ok(vec![])
        }

        async fn get_by_id(&self, _: &str) -> AppResult<Option<scryer_domain::IndexerConfig>> {
            Ok(None)
        }

        async fn create(
            &self,
            _: scryer_domain::IndexerConfig,
        ) -> AppResult<scryer_domain::IndexerConfig> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn touch_last_error(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update(
            &self,
            _: crate::IndexerConfigUpdate,
        ) -> AppResult<scryer_domain::IndexerConfig> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl ImportRepository for TestImportRepo {
        async fn queue_import_request(
            &self,
            _: DownloadSourceIdentity,
            _: String,
            _: String,
        ) -> AppResult<String> {
            Ok(String::new())
        }

        async fn get_import_by_id(&self, _: &str) -> AppResult<Option<ImportRecord>> {
            Ok(None)
        }

        async fn update_import_status(
            &self,
            _: &str,
            status: ImportStatus,
            result_json: Option<String>,
        ) -> AppResult<()> {
            self.status_updates
                .lock()
                .await
                .push(TestImportStatusUpdate(status, result_json));
            Ok(())
        }

        async fn recover_stale_processing_imports(&self, _: i64) -> AppResult<u64> {
            Ok(0)
        }

        async fn recover_stale_processing_imports_for_type(
            &self,
            _: ImportType,
            _: i64,
        ) -> AppResult<u64> {
            Ok(0)
        }

        async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
            Ok(vec![])
        }

        async fn list_pending_imports_for_type(
            &self,
            _: ImportType,
        ) -> AppResult<Vec<ImportRecord>> {
            Ok(vec![])
        }

        async fn list_imports_for_identities(
            &self,
            identities: &[DownloadSourceIdentity],
        ) -> AppResult<Vec<ImportRecord>> {
            Ok(self
                .stored_imports()
                .into_iter()
                .filter(|record| {
                    identities.iter().any(|identity| {
                        record.source_client_id.as_deref().unwrap_or("")
                            == identity.client_id_or_empty()
                            && record.source_system == identity.client_type
                            && record.source_ref == identity.item_id
                    })
                })
                .collect())
        }

        async fn is_already_imported(&self, identity: &DownloadSourceIdentity) -> AppResult<bool> {
            Ok(self.stored_imports().iter().any(|record| {
                record.source_client_id.as_deref().unwrap_or("") == identity.client_id_or_empty()
                    && record.source_system == identity.client_type
                    && record.source_ref == identity.item_id
                    && matches!(
                        record.status,
                        ImportStatus::Completed | ImportStatus::Skipped
                    )
            }))
        }

        async fn list_imports(&self, _: usize) -> AppResult<Vec<ImportRecord>> {
            Ok(vec![])
        }
    }

    #[async_trait]
    impl DomainEventRepository for TestDomainEventRepo {
        async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
            let mut events = self.events.lock().await;
            let sequence = events
                .last()
                .map(|existing| existing.sequence + 1)
                .unwrap_or(1);
            let stored = DomainEvent {
                sequence,
                event_id: event.event_id,
                occurred_at: event.occurred_at,
                actor_user_id: event.actor_user_id,
                title_id: event.title_id,
                facet: event.facet,
                correlation_id: event.correlation_id,
                causation_id: event.causation_id,
                schema_version: event.schema_version,
                stream: event.stream,
                payload: event.payload,
            };
            events.push(stored.clone());
            Ok(stored)
        }

        async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
            let mut stored = Vec::with_capacity(events.len());
            for event in events {
                stored.push(self.append(event).await?);
            }
            Ok(stored)
        }

        async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
            let events = self.events.lock().await;
            Ok(events
                .iter()
                .filter(|event| {
                    filter
                        .after_sequence
                        .is_none_or(|after| event.sequence > after)
                        && filter
                            .before_sequence
                            .is_none_or(|before| event.sequence < before)
                        && filter.title_id.as_ref().is_none_or(|title_id| {
                            event.title_id.as_deref() == Some(title_id.as_str())
                        })
                        && filter
                            .facet
                            .as_ref()
                            .is_none_or(|facet| event.facet.as_ref() == Some(facet))
                        && filter.event_types.as_ref().is_none_or(|event_types| {
                            event_types
                                .iter()
                                .any(|event_type| &event.payload.event_type() == event_type)
                        })
                })
                .cloned()
                .collect())
        }

        async fn count_title_history_page_events(
            &self,
            event_types: Option<&[TitleHistoryEventType]>,
            title_ids: Option<&[String]>,
            download_id: Option<&str>,
        ) -> AppResult<i64> {
            let events = self.events.lock().await;
            Ok(events
                .iter()
                .rev()
                .filter_map(crate::event_views::title_history_record_from_domain_event)
                .filter(|record| {
                    event_types.is_none_or(|values| values.contains(&record.event_type))
                        && title_ids.is_none_or(|values| values.contains(&record.title_id))
                        && download_id
                            .is_none_or(|value| record.download_id.as_deref() == Some(value))
                })
                .count() as i64)
        }

        async fn list_title_history_page_events(
            &self,
            event_types: Option<&[TitleHistoryEventType]>,
            title_ids: Option<&[String]>,
            download_id: Option<&str>,
            limit: usize,
            offset: usize,
        ) -> AppResult<Vec<DomainEvent>> {
            let page_size = if limit == 0 { usize::MAX } else { limit };
            let events = self.events.lock().await;
            Ok(events
                .iter()
                .rev()
                .filter(|event| {
                    crate::event_views::title_history_record_from_domain_event(event).is_some_and(
                        |record| {
                            event_types.is_none_or(|values| values.contains(&record.event_type))
                                && title_ids.is_none_or(|values| values.contains(&record.title_id))
                                && download_id.is_none_or(|value| {
                                    record.download_id.as_deref() == Some(value)
                                })
                        },
                    )
                })
                .skip(offset)
                .take(page_size)
                .cloned()
                .collect())
        }

        async fn list_after_sequence(
            &self,
            after_sequence: i64,
            limit: usize,
        ) -> AppResult<Vec<DomainEvent>> {
            let events = self.events.lock().await;
            Ok(events
                .iter()
                .filter(|event| event.sequence > after_sequence)
                .take(limit)
                .cloned()
                .collect())
        }

        async fn delete_for_title_ids(&self, _title_ids: &[String]) -> AppResult<u32> {
            Ok(0)
        }

        async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64> {
            let offsets = self.subscriber_offsets.lock().await;
            Ok(*offsets.get(subscriber).unwrap_or(&0))
        }

        async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()> {
            let mut offsets = self.subscriber_offsets.lock().await;
            offsets.insert(subscriber.to_string(), sequence);
            Ok(())
        }
    }

    #[async_trait]
    impl TitleRepository for TestTitleRepo {
        async fn list(&self, _: Option<MediaFacet>, _: Option<String>) -> AppResult<Vec<Title>> {
            Ok(self.titles.clone())
        }

        async fn list_by_external_ids(
            &self,
            source: &str,
            values: &[String],
        ) -> AppResult<Vec<Title>> {
            let mut matches = Vec::new();
            let mut seen = HashSet::new();
            for value in values
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            {
                if let Some(title) = self.titles.iter().find(|title| {
                    title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case(source)
                            && external_id.value == value
                    })
                }) && seen.insert(title.id.clone())
                {
                    matches.push(title.clone());
                }
            }
            Ok(matches)
        }

        async fn list_for_matching(
            &self,
            _: Option<MediaFacet>,
            _: Option<String>,
        ) -> AppResult<Vec<Title>> {
            Ok(self.titles.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
            Ok(self.titles.iter().find(|title| title.id == id).cloned())
        }

        async fn get_by_facet_and_slug(
            &self,
            facet: MediaFacet,
            slug: &str,
        ) -> AppResult<Option<Title>> {
            let normalized_slug = slug.trim();
            if normalized_slug.is_empty() {
                return Ok(None);
            }

            let matches = self
                .titles
                .iter()
                .filter(|title| {
                    title.facet == facet
                        && title.slug.as_deref().is_some_and(|candidate| {
                            candidate.trim().eq_ignore_ascii_case(normalized_slug)
                        })
                })
                .cloned()
                .collect::<Vec<_>>();

            match matches.as_slice() {
                [] => Ok(None),
                [title] => Ok(Some(title.clone())),
                _ => Err(AppError::Validation(
                    "multiple titles found for slug lookup".into(),
                )),
            }
        }

        async fn find_by_external_id(&self, _: &str, _: &str) -> AppResult<Option<Title>> {
            Ok(None)
        }

        async fn find_by_external_id_in_facet(
            &self,
            _: MediaFacet,
            _: &str,
            _: &str,
        ) -> AppResult<Option<Title>> {
            Ok(None)
        }

        async fn create_or_get_existing(&self, _: Title) -> AppResult<CreateTitleOutcome> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn create(&self, _: Title) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn list_titles_due_for_hydration(
            &self,
            _: usize,
            _: &[MediaFacet],
        ) -> AppResult<Vec<PendingTitleHydration>> {
            Ok(vec![])
        }

        async fn list_anime_title_ids_missing_anibridge_scoped_external_ids(
            &self,
            _: usize,
        ) -> AppResult<Vec<String>> {
            Ok(vec![])
        }

        async fn mark_title_metadata_hydration_due_now(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn schedule_title_metadata_hydration_retry(
            &self,
            _: &str,
            _: &str,
            _: i64,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn clear_title_metadata_hydration_retry_state(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update_monitored(&self, _: &str, _: bool) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_metadata(
            &self,
            _: &str,
            _: Option<String>,
            _: Option<MediaFacet>,
            _: Option<Vec<String>>,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_title_hydrated_metadata(
            &self,
            _: &str,
            _: TitleMetadataUpdate,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn replace_match_state(
            &self,
            _: &str,
            _: Vec<scryer_domain::ExternalId>,
            _: Vec<String>,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn set_folder_path(&self, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn clear_folder_path(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
            Ok(0)
        }
    }

    #[async_trait]
    impl TitleRepository for MutableTitleRepo {
        async fn list(&self, _: Option<MediaFacet>, _: Option<String>) -> AppResult<Vec<Title>> {
            Ok(self.titles.lock().await.clone())
        }

        async fn list_by_external_ids(
            &self,
            source: &str,
            values: &[String],
        ) -> AppResult<Vec<Title>> {
            let titles = self.titles.lock().await;
            let mut matches = Vec::new();
            let mut seen = HashSet::new();
            for value in values
                .iter()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            {
                if let Some(title) = titles.iter().find(|title| {
                    title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case(source)
                            && external_id.value == value
                    })
                }) && seen.insert(title.id.clone())
                {
                    matches.push(title.clone());
                }
            }
            Ok(matches)
        }

        async fn list_for_matching(
            &self,
            _: Option<MediaFacet>,
            _: Option<String>,
        ) -> AppResult<Vec<Title>> {
            *self.list_for_matching_calls.lock().await += 1;
            Ok(self.titles.lock().await.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
            Ok(self
                .titles
                .lock()
                .await
                .iter()
                .find(|title| title.id == id)
                .cloned())
        }

        async fn get_by_facet_and_slug(
            &self,
            facet: MediaFacet,
            slug: &str,
        ) -> AppResult<Option<Title>> {
            let normalized_slug = slug.trim();
            if normalized_slug.is_empty() {
                return Ok(None);
            }

            let titles = self.titles.lock().await;
            let matches = titles
                .iter()
                .filter(|title| {
                    title.facet == facet
                        && title.slug.as_deref().is_some_and(|candidate| {
                            candidate.trim().eq_ignore_ascii_case(normalized_slug)
                        })
                })
                .cloned()
                .collect::<Vec<_>>();

            match matches.as_slice() {
                [] => Ok(None),
                [title] => Ok(Some(title.clone())),
                _ => Err(AppError::Validation(
                    "multiple titles found for slug lookup".into(),
                )),
            }
        }

        async fn find_by_external_id(&self, _: &str, _: &str) -> AppResult<Option<Title>> {
            Ok(None)
        }

        async fn find_by_external_id_in_facet(
            &self,
            _: MediaFacet,
            _: &str,
            _: &str,
        ) -> AppResult<Option<Title>> {
            Ok(None)
        }

        async fn create_or_get_existing(&self, _: Title) -> AppResult<CreateTitleOutcome> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn create(&self, _: Title) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn list_titles_due_for_hydration(
            &self,
            _: usize,
            _: &[MediaFacet],
        ) -> AppResult<Vec<PendingTitleHydration>> {
            Ok(vec![])
        }

        async fn list_anime_title_ids_missing_anibridge_scoped_external_ids(
            &self,
            _: usize,
        ) -> AppResult<Vec<String>> {
            Ok(vec![])
        }

        async fn mark_title_metadata_hydration_due_now(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn schedule_title_metadata_hydration_retry(
            &self,
            _: &str,
            _: &str,
            _: i64,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn clear_title_metadata_hydration_retry_state(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update_monitored(&self, _: &str, _: bool) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_metadata(
            &self,
            _: &str,
            _: Option<String>,
            _: Option<MediaFacet>,
            _: Option<Vec<String>>,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn update_title_hydrated_metadata(
            &self,
            _: &str,
            _: TitleMetadataUpdate,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn replace_match_state(
            &self,
            _: &str,
            _: Vec<scryer_domain::ExternalId>,
            _: Vec<String>,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not needed in test".into()))
        }

        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn set_folder_path(&self, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn clear_folder_path(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
            Ok(0)
        }
    }

    fn build_app(
        download_submissions: Arc<TestDownloadSubmissionRepo>,
        imports: Arc<TestImportRepo>,
    ) -> AppUseCase {
        build_app_with_title_repo(Arc::new(NullTitleRepository), download_submissions, imports)
    }

    fn build_app_with_title_repo(
        title_repo: Arc<dyn TitleRepository>,
        download_submissions: Arc<TestDownloadSubmissionRepo>,
        imports: Arc<TestImportRepo>,
    ) -> AppUseCase {
        build_app_with_title_repo_and_download_client(
            title_repo,
            Arc::new(NullDownloadClient),
            download_submissions,
            imports,
        )
    }

    fn build_app_with_title_repo_and_download_client(
        title_repo: Arc<dyn TitleRepository>,
        download_client: Arc<dyn DownloadClient>,
        download_submissions: Arc<TestDownloadSubmissionRepo>,
        imports: Arc<TestImportRepo>,
    ) -> AppUseCase {
        let services = AppServices::builder(
            title_repo,
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            Arc::new(TestIndexerConfigRepo),
            Arc::new(NullIndexerClient),
            download_client,
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(crate::null_repositories::NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
        .with_download_submissions(download_submissions)
        .with_imports(imports)
        .with_domain_events(Arc::new(TestDomainEventRepo::default()))
        .build_partial_for_tests();

        AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    fn trigger_user() -> User {
        let mut libraries = HashMap::new();
        let permissions = scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::ManageTitles,
            scryer_domain::LibraryPermission::ResolveImports,
            scryer_domain::LibraryPermission::ManageLibrary,
        ]);
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            libraries.insert(
                scryer_domain::default_library_id_for_facet(&facet),
                permissions,
            );
        }
        User {
            id: "user-1".to_string(),
            username: "user@example.test".to_string(),
            password_hash: None,
            authorization: scryer_domain::UserAuthorization {
                app: scryer_domain::AppPermissionMask::NONE,
                libraries,
                default_library: permissions,
                loaded: true,
            },
        }
    }

    fn build_client_item() -> DownloadQueueItem {
        DownloadQueueItem {
            id: Id::new().0,
            title_id: None,
            episode_id: None,
            title_name: "Restart Recovery Show".to_string(),
            facet: Some("series".to_string()),
            client_id: "client-1".to_string(),
            client_name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            state: DownloadQueueState::Completed,
            progress_percent: 100,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "dl-1".to_string(),
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: vec![],
            tracked_match_type: None,
        }
    }

    fn build_completed_download(
        client_type: &str,
        item_id: &str,
        name: &str,
        dest_dir: &str,
        category: Option<&str>,
    ) -> CompletedDownload {
        CompletedDownload {
            client_type: client_type.to_string(),
            client_id: "client-1".to_string(),
            download_client_item_id: item_id.to_string(),
            name: name.to_string(),
            dest_dir: dest_dir.to_string(),
            category: category.map(str::to_string),
            size_bytes: None,
            completed_at: None,
            parameters: vec![],
        }
    }

    fn build_title(name: &str, facet: MediaFacet, aliases: &[&str]) -> Title {
        Title {
            id: Id::new().0,
            name: name.to_string(),
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
            banner_url: None,
            banner_source_url: None,
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
            aliases: aliases.iter().map(|value| value.to_string()).collect(),
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    #[tokio::test]
    async fn reconstruct_state_recovers_imported_from_completed_import_record() {
        let download_submissions = Arc::new(TestDownloadSubmissionRepo {
            submission: Some(crate::DownloadSubmission {
                title_id: "title-1".to_string(),
                facet: "series".to_string(),
                download_client_id: None,
                download_client_type: "nzbget".to_string(),
                download_client_item_id: "dl-1".to_string(),
                source_hint: None,
                source_kind: None,
                source_title: Some("Restart Recovery Show".to_string()),
                request_signature: None,
                scope: crate::SubmissionScope::Title,
            }),
            tracked_state: None,
            tracked_state_updates: Arc::new(Mutex::new(vec![])),
            recorded_submissions: Arc::new(Mutex::new(vec![])),
        });
        let imports = Arc::new(TestImportRepo {
            import_record: Some(ImportRecord {
                id: Id::new().0,
                source_client_id: Some("client-1".to_string()),
                source_system: "nzbget".to_string(),
                source_ref: "dl-1".to_string(),
                import_type: ImportType::SeriesDownload,
                status: ImportStatus::Completed,
                payload_json: "{}".to_string(),
                result_json: None,
                started_at: None,
                finished_at: None,
                created_at: "now".to_string(),
                updated_at: "now".to_string(),
            }),
            ..Default::default()
        });
        let app = build_app(download_submissions.clone(), imports);
        let mut tracker = TrackedDownloadService::new();

        tracker.track(&app, build_client_item()).await;

        let tracked = tracker.find("client-1:dl-1").expect("tracked download");
        assert_eq!(tracked.state, TrackedDownloadState::Imported);
        assert_eq!(
            download_submissions
                .tracked_state_updates
                .lock()
                .await
                .as_slice(),
            ["imported"]
        );
    }

    #[tokio::test]
    async fn persist_terminal_state_returns_false_when_repository_write_fails() {
        #[derive(Default)]
        struct FailingDownloadSubmissionRepo;

        #[async_trait]
        impl DownloadSubmissionRepository for FailingDownloadSubmissionRepo {
            async fn record_submission(&self, _: crate::DownloadSubmission) -> AppResult<()> {
                Ok(())
            }

            async fn find_by_client_item_id(
                &self,
                _: &DownloadSourceIdentity,
            ) -> AppResult<Option<crate::DownloadSubmission>> {
                Ok(None)
            }

            async fn list_for_client_items(
                &self,
                _: &[DownloadSourceIdentity],
            ) -> AppResult<Vec<crate::DownloadSubmission>> {
                Ok(vec![])
            }

            async fn list_for_title(&self, _: &str) -> AppResult<Vec<crate::DownloadSubmission>> {
                Ok(vec![])
            }

            async fn find_by_title_and_request_signature(
                &self,
                _: &str,
                _: &str,
            ) -> AppResult<Option<crate::DownloadSubmission>> {
                Ok(None)
            }

            async fn delete_for_title(&self, _: &str) -> AppResult<()> {
                Ok(())
            }

            async fn delete_by_client_item_id(&self, _: &DownloadSourceIdentity) -> AppResult<()> {
                Ok(())
            }

            async fn update_tracked_state(
                &self,
                _: &DownloadSourceIdentity,
                _: &str,
            ) -> AppResult<()> {
                Err(AppError::Repository("boom".into()))
            }

            async fn get_tracked_state(
                &self,
                _: &DownloadSourceIdentity,
            ) -> AppResult<Option<String>> {
                Ok(None)
            }
        }

        let services = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            Arc::new(TestIndexerConfigRepo),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(crate::null_repositories::NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
        .with_download_submissions(Arc::new(FailingDownloadSubmissionRepo))
        .with_imports(Arc::new(TestImportRepo::default()))
        .build_partial_for_tests();

        let app = AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        );

        let mut tracker = TrackedDownloadService::new();
        tracker.track(&app, build_client_item()).await;

        assert!(
            tracker.find("client-1:dl-1").is_some(),
            "tracked download should exist before persistence attempt"
        );

        let persisted = tracker
            .persist_terminal_state(&app, "client-1:dl-1", TrackedDownloadState::Failed)
            .await;

        assert!(!persisted, "persistence should report failure");
        assert!(
            tracker.find("client-1:dl-1").is_some(),
            "tracked download should remain cached when persistence fails"
        );
    }

    #[tokio::test]
    async fn completed_episode_download_uses_title_parse_to_become_import_pending() {
        let title = build_title(
            "House of Ravens",
            MediaFacet::Anime,
            &["RAVENCOURT The Last Regent"],
        );
        let title_repo = Arc::new(TestTitleRepo {
            titles: vec![title.clone()],
        });
        let download_submissions = Arc::new(TestDownloadSubmissionRepo {
            submission: None,
            tracked_state: None,
            tracked_state_updates: Arc::new(Mutex::new(vec![])),
            recorded_submissions: Arc::new(Mutex::new(vec![])),
        });
        let imports = Arc::new(TestImportRepo::default());
        let tempdir = tempfile::tempdir().expect("tempdir");
        let completed_dir = tempdir
            .path()
            .join("RAVENCOURT.The.Last.Regent.S01E18.1080p.WEB-DL");
        std::fs::create_dir_all(&completed_dir).expect("create completed download dir");
        let download_client = Arc::new(TestDownloadClient {
            completed_downloads: Arc::new(Mutex::new(vec![build_completed_download(
                "weaver",
                "job-1",
                "RAVENCOURT.The.Last.Regent.S01E18.1080p.WEB-DL",
                completed_dir.to_string_lossy().as_ref(),
                Some("anime"),
            )])),
            ..Default::default()
        });
        let app = build_app_with_title_repo_and_download_client(
            title_repo,
            download_client,
            download_submissions,
            imports,
        );
        let mut tracker = TrackedDownloadService::new();
        let mut item = build_client_item();
        item.client_type = "weaver".to_string();
        item.client_name = "weaver".to_string();
        item.download_client_item_id = "job-1".to_string();
        item.title_name =
            "RAVENCOURT.The.Last.Regent.S01E18.1080p.WEB-DL".to_string();
        item.facet = Some("anime".to_string());
        item.is_scryer_origin = false;

        tracker.track(&app, item).await;

        let tracked = tracker.find("client-1:job-1").expect("tracked download");
        assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));
        assert_eq!(tracked.match_type, TitleMatchType::TitleParse);

        let tracked = tracker
            .find_mut("client-1:job-1")
            .expect("tracked download mut");
        crate::completed_download_handler::check(&app, tracked).await;

        assert_eq!(tracked.state, TrackedDownloadState::ImportPending);
        assert!(tracked.status_messages.is_empty());
    }

    #[tokio::test]
    async fn tracked_download_resolution_marks_embedded_external_id_matches_as_id_only() {
        let mut title = build_title("Paper Lantern", MediaFacet::Movie, &[]);
        title.external_ids.push(scryer_domain::ExternalId {
            source: "imdb".to_string(),
            value: "tt2388725".to_string(),
        });
        let title_repo = Arc::new(TestTitleRepo {
            titles: vec![title.clone()],
        });
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app_with_title_repo(title_repo, download_submissions, imports);
        let mut tracker = TrackedDownloadService::new();
        let mut item = build_client_item();
        item.client_type = "weaver".to_string();
        item.client_name = "weaver".to_string();
        item.download_client_item_id = "job-imdb".to_string();
        item.title_name = "Paper.Lantern.2012.[tt2388725].1080p.BluRay.x264-GRP".to_string();
        item.facet = Some("movie".to_string());
        item.is_scryer_origin = false;

        tracker.track(&app, item).await;

        let tracked = tracker.find("client-1:job-imdb").expect("tracked download");
        assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));
        assert_eq!(tracked.match_type, TitleMatchType::IdOnly);
    }

    #[tokio::test]
    async fn assigning_title_to_completed_blocked_download_keeps_manual_import_actionable() {
        let title = build_title("Paper Lantern", MediaFacet::Movie, &[]);
        let title_repo = Arc::new(TestTitleRepo {
            titles: vec![title.clone()],
        });
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let tempdir = tempfile::tempdir().expect("tempdir");
        let completed_dir = tempdir.path().join("4f8e2c7a91b6d3e0");
        std::fs::create_dir_all(&completed_dir).expect("create completed download dir");
        let download_client = Arc::new(TestDownloadClient {
            completed_downloads: Arc::new(Mutex::new(vec![build_completed_download(
                "weaver",
                "job-manual-movie",
                "4f8e2c7a91b6d3e0",
                completed_dir.to_string_lossy().as_ref(),
                Some("movie"),
            )])),
            ..Default::default()
        });
        let app = build_app_with_title_repo_and_download_client(
            title_repo,
            download_client,
            download_submissions,
            imports,
        );
        let mut tracker = TrackedDownloadService::new();

        let mut item = build_client_item();
        item.client_type = "weaver".to_string();
        item.client_name = "weaver".to_string();
        item.download_client_item_id = "job-manual-movie".to_string();
        item.title_name = "4f8e2c7a91b6d3e0".to_string();
        item.facet = Some("movie".to_string());
        item.is_scryer_origin = false;

        tracker.track(&app, item).await;
        let tracked = tracker
            .find_mut("client-1:job-manual-movie")
            .expect("tracked download mut");
        crate::completed_download_handler::check(&app, tracked).await;
        assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
        assert!(tracked.title_id.is_none());

        let tracked = tracker
            .find_mut("client-1:job-manual-movie")
            .expect("tracked download mut");
        assign_title_to_tracked_download(&app, tracked, &title).await;

        assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
        assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));
        assert_eq!(tracked.match_type, TitleMatchType::Submission);
        assert!(!tracked.import_attempted);

        crate::completed_download_handler::check(&app, tracked).await;
        assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
    }

    #[tokio::test]
    async fn repeated_unmatched_snapshot_uses_cached_matcher_until_title_event_invalidates_it() {
        let titles = Arc::new(Mutex::new(Vec::new()));
        let list_for_matching_calls = Arc::new(Mutex::new(0usize));
        let title_repo = Arc::new(MutableTitleRepo {
            titles: titles.clone(),
            list_for_matching_calls: list_for_matching_calls.clone(),
        });
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app_with_title_repo(title_repo, download_submissions, imports);
        let mut tracker = TrackedDownloadService::new();

        let mut initial = build_client_item();
        initial.client_type = "weaver".to_string();
        initial.client_name = "weaver".to_string();
        initial.download_client_item_id = "job-manual-movie-reresolve".to_string();
        initial.title_name = "Paper Lantern".to_string();
        initial.facet = Some("movie".to_string());
        initial.is_scryer_origin = false;

        tracker.track(&app, initial).await;
        let tracked = tracker
            .find("client-1:job-manual-movie-reresolve")
            .expect("tracked download");
        assert!(tracked.title_id.is_none());
        assert_eq!(tracked.match_type, TitleMatchType::Unmatched);
        assert_eq!(*list_for_matching_calls.lock().await, 1);

        let mut unchanged = build_client_item();
        unchanged.client_type = "weaver".to_string();
        unchanged.client_name = "weaver".to_string();
        unchanged.download_client_item_id = "job-manual-movie-reresolve".to_string();
        unchanged.title_name = "Paper Lantern".to_string();
        unchanged.facet = Some("movie".to_string());
        unchanged.is_scryer_origin = false;

        tracker.track(&app, unchanged).await;

        let tracked = tracker
            .find("client-1:job-manual-movie-reresolve")
            .expect("tracked download");
        assert!(tracked.title_id.is_none());
        assert_eq!(tracked.match_type, TitleMatchType::Unmatched);
        assert_eq!(
            *list_for_matching_calls.lock().await,
            1,
            "unchanged unmatched polls should reuse the cached matcher"
        );

        let title = build_title("Paper Lantern", MediaFacet::Movie, &[]);
        titles.lock().await.push(title.clone());
        app.append_domain_event(crate::domain_events::new_title_domain_event(
            None,
            &title,
            scryer_domain::DomainEventPayload::TitleUpdated(scryer_domain::TitleUpdatedEventData {
                title: crate::domain_events::title_context_snapshot(&title),
            }),
        ))
        .await
        .expect("invalidate matcher");

        let mut updated = build_client_item();
        updated.client_type = "weaver".to_string();
        updated.client_name = "weaver".to_string();
        updated.download_client_item_id = "job-manual-movie-reresolve".to_string();
        updated.title_name = "Paper Lantern".to_string();
        updated.facet = Some("movie".to_string());
        updated.is_scryer_origin = false;

        tracker.track(&app, updated).await;

        let tracked = tracker
            .find("client-1:job-manual-movie-reresolve")
            .expect("tracked download");
        assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));
        assert_eq!(tracked.match_type, TitleMatchType::TitleParse);
        assert_eq!(
            *list_for_matching_calls.lock().await,
            2,
            "title events should invalidate the cached matcher"
        );
    }

    #[tokio::test]
    async fn unchanged_unmatched_snapshot_does_not_reresolve_every_poll() {
        let title_repo = Arc::new(TestTitleRepo::default());
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app_with_title_repo(title_repo, download_submissions.clone(), imports);
        let mut tracker = TrackedDownloadService::new();

        let mut initial = build_client_item();
        initial.client_type = "weaver".to_string();
        initial.client_name = "weaver".to_string();
        initial.download_client_item_id = "job-unmatched-repeat".to_string();
        initial.title_name = "Paper Lantern".to_string();
        initial.facet = Some("movie".to_string());
        initial.is_scryer_origin = false;

        tracker.track(&app, initial.clone()).await;
        tracker.track(&app, initial).await;

        let recorded = download_submissions.recorded_submissions.lock().await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].download_client_item_id,
            "job-unmatched-repeat".to_string()
        );
    }

    #[tokio::test]
    async fn assigning_title_to_blocked_download_keeps_manual_import_actionable_even_if_client_is_still_downloading()
     {
        let title = build_title("Paper Lantern", MediaFacet::Movie, &[]);
        let title_repo = Arc::new(TestTitleRepo {
            titles: vec![title.clone()],
        });
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app_with_title_repo(title_repo, download_submissions, imports);
        let mut tracker = TrackedDownloadService::new();

        let mut item = build_client_item();
        item.client_type = "weaver".to_string();
        item.client_name = "weaver".to_string();
        item.download_client_item_id = "job-manual-movie-downloading".to_string();
        item.title_name = "Paper Lantern".to_string();
        item.facet = Some("movie".to_string());
        item.state = DownloadQueueState::Downloading;
        item.is_scryer_origin = false;

        tracker.track(&app, item).await;
        let tracked = tracker
            .find_mut("client-1:job-manual-movie-downloading")
            .expect("tracked download mut");
        tracked.state = TrackedDownloadState::ImportBlocked;
        tracked.match_type = TitleMatchType::Unmatched;
        tracked.title_id = None;

        assign_title_to_tracked_download(&app, tracked, &title).await;

        assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
        assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));
        assert_eq!(tracked.match_type, TitleMatchType::Submission);
        assert!(!tracked.import_attempted);

        crate::completed_download_handler::check(&app, tracked).await;
        assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
    }

    #[tokio::test]
    async fn track_reresolves_when_scryer_metadata_arrives_on_later_snapshot() {
        let title = build_title("House of Ravens", MediaFacet::Anime, &[]);
        let title_repo = Arc::new(TestTitleRepo {
            titles: vec![title.clone()],
        });
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app_with_title_repo(title_repo, download_submissions, imports);
        let mut tracker = TrackedDownloadService::new();

        let mut initial = build_client_item();
        initial.client_type = "weaver".to_string();
        initial.client_name = "weaver".to_string();
        initial.download_client_item_id = "job-2".to_string();
        initial.title_id = None;
        initial.facet = Some("anime".to_string());
        initial.title_name = "RAVENCOURT".to_string();
        initial.is_scryer_origin = false;

        tracker.track(&app, initial).await;
        let tracked = tracker.find("client-1:job-2").expect("tracked download");
        assert_eq!(tracked.match_type, TitleMatchType::Unmatched);
        assert!(tracked.title_id.is_none());

        let mut updated = build_client_item();
        updated.client_type = "weaver".to_string();
        updated.client_name = "weaver".to_string();
        updated.download_client_item_id = "job-2".to_string();
        updated.title_id = Some(title.id.clone());
        updated.facet = Some("anime".to_string());
        updated.title_name = "RAVENCOURT".to_string();
        updated.is_scryer_origin = true;

        tracker.track(&app, updated).await;

        let tracked = tracker.find("client-1:job-2").expect("tracked download");
        assert_eq!(tracked.match_type, TitleMatchType::ClientParameter);
        assert_eq!(tracked.title_id.as_deref(), Some(title.id.as_str()));
        assert!(tracked.client_item.is_scryer_origin);
    }

    #[tokio::test]
    async fn track_reresolves_when_facet_hint_arrives_on_later_snapshot() {
        let anime_title = build_title("Tidal Quest", MediaFacet::Anime, &[]);
        let series_title = build_title("Tidal Quest", MediaFacet::Series, &[]);
        let title_repo = Arc::new(TestTitleRepo {
            titles: vec![anime_title.clone(), series_title],
        });
        let download_submissions = Arc::new(TestDownloadSubmissionRepo::default());
        let imports = Arc::new(TestImportRepo::default());
        let app = build_app_with_title_repo(title_repo, download_submissions, imports);
        let mut tracker = TrackedDownloadService::new();

        let mut initial = build_client_item();
        initial.client_type = "weaver".to_string();
        initial.client_name = "weaver".to_string();
        initial.download_client_item_id = "job-facet-reresolve".to_string();
        initial.title_name = "Tidal.Quest.S01E01.1080p.WEB-DL".to_string();
        initial.facet = None;
        initial.is_scryer_origin = false;

        tracker.track(&app, initial).await;

        let tracked = tracker
            .find("client-1:job-facet-reresolve")
            .expect("tracked download");
        assert_eq!(tracked.match_type, TitleMatchType::Unmatched);
        assert!(tracked.title_id.is_none());

        let mut updated = build_client_item();
        updated.client_type = "weaver".to_string();
        updated.client_name = "weaver".to_string();
        updated.download_client_item_id = "job-facet-reresolve".to_string();
        updated.title_name = "Tidal.Quest.S01E01.1080p.WEB-DL".to_string();
        updated.facet = Some("anime".to_string());
        updated.is_scryer_origin = false;

        tracker.track(&app, updated).await;

        let tracked = tracker
            .find("client-1:job-facet-reresolve")
            .expect("tracked download");
        assert_eq!(tracked.match_type, TitleMatchType::TitleParse);
        assert_eq!(tracked.title_id.as_deref(), Some(anime_title.id.as_str()));
    }

    #[test]
    fn update_trackable_preserves_in_flight_import_states() {
        let mut tracker = TrackedDownloadService::new();

        for (suffix, state) in [
            ("pending", TrackedDownloadState::ImportPending),
            ("importing", TrackedDownloadState::Importing),
            ("failed", TrackedDownloadState::FailedPending),
        ] {
            tracker.cache.insert(
                format!("client-1:{suffix}"),
                TrackedDownload {
                    id: format!("client-1:{suffix}"),
                    client_id: "client-1".to_string(),
                    client_type: "nzbget".to_string(),
                    client_item: build_client_item(),
                    state,
                    status: TrackedDownloadStatus::Ok,
                    status_messages: Vec::new(),
                    title_id: None,
                    facet: Some("series".to_string()),
                    source_title: None,
                    indexer: None,
                    added_at: None,
                    notified_manual_interaction: false,
                    match_type: TitleMatchType::Unmatched,
                    is_trackable: true,
                    import_attempted: false,
                    path_missing_since: None,
                },
            );
        }

        tracker.update_trackable(&HashSet::new());

        assert!(
            tracker
                .find("client-1:pending")
                .is_some_and(|td| td.is_trackable)
        );
        assert!(
            tracker
                .find("client-1:importing")
                .is_some_and(|td| td.is_trackable)
        );
        assert!(
            tracker
                .find("client-1:failed")
                .is_some_and(|td| td.is_trackable)
        );
    }

    #[test]
    fn failed_download_check_preempts_import_pending_state() {
        let mut client_item = build_client_item();
        client_item.state = DownloadQueueState::Failed;
        client_item.attention_reason = Some("health below critical".to_string());
        let mut tracked = TrackedDownload {
            id: "client-1:failed-import-pending".to_string(),
            client_id: "client-1".to_string(),
            client_type: "nzbget".to_string(),
            client_item,
            state: TrackedDownloadState::ImportPending,
            status: TrackedDownloadStatus::Ok,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("series".to_string()),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
            is_trackable: true,
            import_attempted: false,
            path_missing_since: None,
        };

        crate::failed_download_handler::check(&mut tracked);

        assert_eq!(tracked.state, TrackedDownloadState::FailedPending);
        assert_eq!(tracked.status, TrackedDownloadStatus::Error);
    }

    #[tokio::test]
    async fn queue_manual_import_rejects_failed_source_job() {
        let mut failed_item = build_client_item();
        failed_item.client_type = "weaver".to_string();
        failed_item.client_name = "weaver".to_string();
        failed_item.download_client_item_id = "job-failed".to_string();
        failed_item.state = DownloadQueueState::Failed;
        failed_item.attention_reason = Some("health below critical".to_string());

        let download_client = Arc::new(TestDownloadClient {
            recent_activity: Arc::new(Mutex::new(vec![failed_item])),
            ..Default::default()
        });
        let app = build_app_with_title_repo_and_download_client(
            Arc::new(NullTitleRepository),
            download_client,
            Arc::new(TestDownloadSubmissionRepo::default()),
            Arc::new(TestImportRepo::default()),
        );

        let result = app
            .queue_manual_import(
                &trigger_user(),
                None,
                Some("client-1".to_string()),
                "weaver".to_string(),
                "job-failed".to_string(),
                None,
            )
            .await;

        assert!(matches!(
            result,
            Err(AppError::Validation(message)) if message.contains("source_job_failed")
        ));
    }

    #[tokio::test]
    async fn preview_manual_import_rejects_failed_source_job() {
        let mut failed_item = build_client_item();
        failed_item.client_type = "weaver".to_string();
        failed_item.client_name = "weaver".to_string();
        failed_item.download_client_item_id = "job-failed-preview".to_string();
        failed_item.state = DownloadQueueState::Failed;
        failed_item.attention_reason = Some("health below critical".to_string());

        let download_client = Arc::new(TestDownloadClient {
            recent_activity: Arc::new(Mutex::new(vec![failed_item])),
            ..Default::default()
        });
        let mut title = build_title("Manual Import", MediaFacet::Movie, &[]);
        title.id = "title-1".to_string();
        let app = build_app_with_title_repo_and_download_client(
            Arc::new(TestTitleRepo {
                titles: vec![title],
            }),
            download_client,
            Arc::new(TestDownloadSubmissionRepo::default()),
            Arc::new(TestImportRepo::default()),
        );

        let result = crate::preview_manual_import(
            &app,
            &trigger_user(),
            Some("client-1"),
            "job-failed-preview",
            "title-1",
        )
        .await;

        assert!(matches!(
            result,
            Err(AppError::Validation(message)) if message.contains("source_job_failed")
        ));
    }

    #[tokio::test]
    async fn failed_source_invalidates_active_manual_import_request() {
        let payload = crate::ManualImportRequestPayload {
            requested_by_user_id: Some("user-1".to_string()),
            title_id: Some("title-1".to_string()),
            download_client_item_id: "job-active-manual".to_string(),
            client_id: Some("client-1".to_string()),
            client_type: "weaver".to_string(),
            files: Vec::new(),
            requested_at: Utc::now().to_rfc3339(),
        };
        let imports = Arc::new(TestImportRepo {
            import_record: Some(ImportRecord {
                id: "import-1".to_string(),
                source_client_id: Some("client-1".to_string()),
                source_system: "weaver".to_string(),
                source_ref: "job-active-manual".to_string(),
                import_type: ImportType::ManualImport,
                status: ImportStatus::Pending,
                payload_json: serde_json::to_string(&payload).expect("serialize payload"),
                result_json: None,
                started_at: None,
                finished_at: None,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            }),
            ..Default::default()
        });
        let app = build_app(
            Arc::new(TestDownloadSubmissionRepo::default()),
            imports.clone(),
        );
        let mut client_item = build_client_item();
        client_item.client_type = "weaver".to_string();
        client_item.download_client_item_id = "job-active-manual".to_string();
        let tracked = TrackedDownload {
            id: "client-1:job-active-manual".to_string(),
            client_id: "client-1".to_string(),
            client_type: "weaver".to_string(),
            client_item,
            state: TrackedDownloadState::FailedPending,
            status: TrackedDownloadStatus::Error,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("series".to_string()),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
            is_trackable: true,
            import_attempted: false,
            path_missing_since: None,
        };

        crate::fail_active_manual_import_for_source(&app, &tracked, "health below critical").await;

        let updates = imports.status_updates.lock().await;
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, ImportStatus::Failed);
        assert!(
            updates[0]
                .1
                .as_deref()
                .is_some_and(|json| json.contains("source_job_failed"))
        );
    }

    #[tokio::test]
    async fn queue_manual_import_reuses_only_matching_client_request() {
        let payload_other = crate::ManualImportRequestPayload {
            requested_by_user_id: Some("user-1".to_string()),
            title_id: Some("title-1".to_string()),
            download_client_item_id: "job-shared".to_string(),
            client_id: Some("client-2".to_string()),
            client_type: "weaver".to_string(),
            files: Vec::new(),
            requested_at: Utc::now().to_rfc3339(),
        };
        let payload_match = crate::ManualImportRequestPayload {
            client_id: Some("client-1".to_string()),
            ..payload_other.clone()
        };
        let imports = Arc::new(TestImportRepo {
            import_records: vec![
                ImportRecord {
                    id: "import-other".to_string(),
                    source_client_id: Some("client-2".to_string()),
                    source_system: "weaver".to_string(),
                    source_ref: "job-shared".to_string(),
                    import_type: ImportType::ManualImport,
                    status: ImportStatus::Pending,
                    payload_json: serde_json::to_string(&payload_other)
                        .expect("serialize other payload"),
                    result_json: None,
                    started_at: None,
                    finished_at: None,
                    created_at: Utc::now().to_rfc3339(),
                    updated_at: Utc::now().to_rfc3339(),
                },
                ImportRecord {
                    id: "import-match".to_string(),
                    source_client_id: Some("client-1".to_string()),
                    source_system: "weaver".to_string(),
                    source_ref: "job-shared".to_string(),
                    import_type: ImportType::ManualImport,
                    status: ImportStatus::Pending,
                    payload_json: serde_json::to_string(&payload_match)
                        .expect("serialize matching payload"),
                    result_json: None,
                    started_at: None,
                    finished_at: None,
                    created_at: Utc::now().to_rfc3339(),
                    updated_at: (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339(),
                },
            ],
            ..Default::default()
        });
        let download_client = Arc::new(TestDownloadClient {
            recent_activity: Arc::new(Mutex::new(vec![DownloadQueueItem {
                client_id: "client-1".to_string(),
                client_name: "weaver".to_string(),
                client_type: "weaver".to_string(),
                download_client_item_id: "job-shared".to_string(),
                state: DownloadQueueState::Completed,
                ..build_client_item()
            }])),
            ..Default::default()
        });
        let app = build_app_with_title_repo_and_download_client(
            Arc::new(NullTitleRepository),
            download_client,
            Arc::new(TestDownloadSubmissionRepo::default()),
            imports,
        );

        let import_id = app
            .queue_manual_import(
                &trigger_user(),
                None,
                Some("client-1".to_string()),
                "weaver".to_string(),
                "job-shared".to_string(),
                None,
            )
            .await
            .expect("manual import should reuse matching request");

        assert_eq!(import_id, "import-match");
    }

    #[tokio::test]
    async fn failed_source_invalidates_only_matching_client_request() {
        let payload_other = crate::ManualImportRequestPayload {
            requested_by_user_id: Some("user-1".to_string()),
            title_id: Some("title-1".to_string()),
            download_client_item_id: "job-shared".to_string(),
            client_id: Some("client-2".to_string()),
            client_type: "weaver".to_string(),
            files: Vec::new(),
            requested_at: Utc::now().to_rfc3339(),
        };
        let payload_match = crate::ManualImportRequestPayload {
            client_id: Some("client-1".to_string()),
            ..payload_other.clone()
        };
        let imports = Arc::new(TestImportRepo {
            import_records: vec![
                ImportRecord {
                    id: "import-other".to_string(),
                    source_client_id: Some("client-2".to_string()),
                    source_system: "weaver".to_string(),
                    source_ref: "job-shared".to_string(),
                    import_type: ImportType::ManualImport,
                    status: ImportStatus::Pending,
                    payload_json: serde_json::to_string(&payload_other)
                        .expect("serialize other payload"),
                    result_json: None,
                    started_at: None,
                    finished_at: None,
                    created_at: Utc::now().to_rfc3339(),
                    updated_at: Utc::now().to_rfc3339(),
                },
                ImportRecord {
                    id: "import-match".to_string(),
                    source_client_id: Some("client-1".to_string()),
                    source_system: "weaver".to_string(),
                    source_ref: "job-shared".to_string(),
                    import_type: ImportType::ManualImport,
                    status: ImportStatus::Pending,
                    payload_json: serde_json::to_string(&payload_match)
                        .expect("serialize matching payload"),
                    result_json: None,
                    started_at: None,
                    finished_at: None,
                    created_at: Utc::now().to_rfc3339(),
                    updated_at: (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339(),
                },
            ],
            ..Default::default()
        });
        let app = build_app(
            Arc::new(TestDownloadSubmissionRepo::default()),
            imports.clone(),
        );
        let tracked = TrackedDownload {
            id: "client-1:job-shared".to_string(),
            client_id: "client-1".to_string(),
            client_type: "weaver".to_string(),
            client_item: DownloadQueueItem {
                client_id: "client-1".to_string(),
                client_name: "weaver".to_string(),
                client_type: "weaver".to_string(),
                download_client_item_id: "job-shared".to_string(),
                ..build_client_item()
            },
            state: TrackedDownloadState::FailedPending,
            status: TrackedDownloadStatus::Error,
            status_messages: Vec::new(),
            title_id: Some("title-1".to_string()),
            facet: Some("series".to_string()),
            source_title: None,
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: TitleMatchType::Submission,
            is_trackable: true,
            import_attempted: false,
            path_missing_since: None,
        };

        crate::fail_active_manual_import_for_source(&app, &tracked, "health below critical").await;

        let updates = imports.status_updates.lock().await;
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, ImportStatus::Failed);
        assert!(
            updates[0]
                .1
                .as_deref()
                .is_some_and(|json| json.contains("\"import_id\":\"import-match\""))
        );
    }
}
