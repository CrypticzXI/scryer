use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ProviderCatalogFamily {
    Subtitle,
    Notification,
    Indexer,
    DownloadClient,
    ArchiveExtractor,
}

impl ProviderCatalogFamily {
    pub const fn all() -> [Self; 5] {
        [
            Self::Subtitle,
            Self::Notification,
            Self::Indexer,
            Self::DownloadClient,
            Self::ArchiveExtractor,
        ]
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Subtitle => "subtitle",
            Self::Notification => "notification",
            Self::Indexer => "indexer",
            Self::DownloadClient => "download_client",
            Self::ArchiveExtractor => "archive_extractor",
        }
    }
}

#[derive(Clone)]
pub struct AppRuntimeEventState {
    pub domain_event_broadcast: broadcast::Sender<i64>,
    /// Wake-only high-water hints for the notification dispatcher. Send-side filtering keeps
    /// operational bursts from waking it, while persisted filtered replay remains authoritative.
    pub notification_event_broadcast: broadcast::Sender<i64>,
    pub import_history_broadcast: broadcast::Sender<()>,
    pub indexers_changed_broadcast: broadcast::Sender<()>,
    pub provider_catalog_changed_broadcast: broadcast::Sender<Vec<ProviderCatalogFamily>>,
    pub settings_changed_broadcast: broadcast::Sender<Vec<String>>,
}

#[derive(Clone)]
pub struct AppRuntimeCatalogState {
    /// Serializes profile validation/reference writes with catalog removal for
    /// all `AppUseCase` clones sharing this runtime. This does not coordinate
    /// separate processes or replicas; multi-process profile mutations require
    /// a shared database advisory/transaction lock before they are supported.
    pub quality_profile_reference_lock: Arc<tokio::sync::Mutex<()>>,
    pub(crate) monitored_title_matcher:
        Arc<RwLock<crate::import_title_resolution::MonitoredTitleMatcherCache>>,
    pub poster_wake: Arc<tokio::sync::Notify>,
    pub fanart_wake: Arc<tokio::sync::Notify>,
    pub(crate) title_hydration_wake: Arc<tokio::sync::Notify>,
    pub(crate) title_recommendation_refresh_queue:
        Arc<tokio::sync::Mutex<crate::catalog_workflow::TitleRecommendationRefreshQueue>>,
    pub(crate) title_recommendation_refresh_wake: Arc<tokio::sync::Notify>,
    pub image_processing_limit: Arc<Semaphore>,
    pub title_image_maintenance_lock: Arc<tokio::sync::RwLock<()>>,
    pub title_image_cache_clear_scheduled: Arc<std::sync::atomic::AtomicBool>,
}

pub const DOWNLOAD_QUEUE_SNAPSHOT_STALE_AFTER: chrono::Duration = chrono::Duration::seconds(30);
pub const DOWNLOAD_QUEUE_SNAPSHOT_COALESCE_WINDOW: std::time::Duration =
    std::time::Duration::from_millis(300);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadQueueSync {
    pub revision: u64,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct DownloadQueueSnapshot {
    pub items: Arc<[DownloadQueueItem]>,
    pub revision: u64,
    pub updated_at: Option<DateTime<Utc>>,
    pub ready: bool,
    pub refresh_error: Option<Arc<str>>,
}

impl DownloadQueueSnapshot {
    pub fn stale_at(&self, now: DateTime<Utc>) -> bool {
        !self.ready
            || self.refresh_error.is_some()
            || self.updated_at.is_none_or(|updated_at| {
                now.signed_duration_since(updated_at) > DOWNLOAD_QUEUE_SNAPSHOT_STALE_AFTER
            })
    }
}

#[derive(Debug)]
pub(crate) struct DownloadQueueReadModel {
    pub(crate) revision: u64,
    pub(crate) items: Arc<[DownloadQueueItem]>,
    pub(crate) title_library_ids: Arc<HashMap<String, String>>,
    pub(crate) orderings:
        tokio::sync::RwLock<HashMap<crate::types::DownloadHistorySort, Arc<[usize]>>>,
    pub(crate) legacy_ordering: tokio::sync::OnceCell<Arc<[usize]>>,
}

impl DownloadQueueReadModel {
    pub(crate) fn new(
        revision: u64,
        items: Arc<[DownloadQueueItem]>,
        title_library_ids: HashMap<String, String>,
    ) -> Self {
        Self {
            revision,
            items,
            title_library_ids: Arc::new(title_library_ids),
            orderings: tokio::sync::RwLock::new(HashMap::new()),
            legacy_ordering: tokio::sync::OnceCell::new(),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct DownloadQueueReadModelCache {
    pub(crate) current: Arc<tokio::sync::RwLock<Option<Arc<DownloadQueueReadModel>>>>,
    pub(crate) build_lock: Arc<tokio::sync::Mutex<()>>,
}

struct PendingDownloadQueueSnapshot {
    items: Vec<DownloadQueueItem>,
    positions: HashMap<(String, String), usize>,
    updated_at: DateTime<Utc>,
    clear_refresh_error: bool,
}

#[derive(Clone)]
pub struct DownloadQueueSnapshotCache {
    state: Arc<tokio::sync::RwLock<DownloadQueueSnapshot>>,
    pending: Arc<tokio::sync::Mutex<Option<PendingDownloadQueueSnapshot>>>,
    commit_scheduled: Arc<std::sync::atomic::AtomicBool>,
    sync_tx: tokio::sync::watch::Sender<DownloadQueueSync>,
}

impl Default for DownloadQueueSnapshotCache {
    fn default() -> Self {
        let sync = DownloadQueueSync {
            revision: 0,
            updated_at: None,
        };
        let (sync_tx, _) = tokio::sync::watch::channel(sync);
        Self {
            state: Arc::new(tokio::sync::RwLock::new(DownloadQueueSnapshot {
                items: Arc::from(Vec::<DownloadQueueItem>::new()),
                revision: 0,
                updated_at: None,
                ready: false,
                refresh_error: None,
            })),
            pending: Arc::new(tokio::sync::Mutex::new(None)),
            commit_scheduled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            sync_tx,
        }
    }
}

impl DownloadQueueSnapshotCache {
    pub async fn snapshot(&self) -> DownloadQueueSnapshot {
        self.state.read().await.clone()
    }

    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<DownloadQueueSync> {
        self.sync_tx.subscribe()
    }

    pub async fn stage_success(&self, items: Vec<DownloadQueueItem>) {
        self.stage_snapshot(items, true).await;
    }

    pub async fn stage_partial_success(&self, items: Vec<DownloadQueueItem>) {
        self.stage_snapshot(items, false).await;
    }

    async fn stage_snapshot(&self, items: Vec<DownloadQueueItem>, clear_refresh_error: bool) {
        let (items, positions) = index_download_queue_items(items);
        let item_count = items.len();
        *self.pending.lock().await = Some(PendingDownloadQueueSnapshot {
            items,
            positions,
            updated_at: Utc::now(),
            clear_refresh_error,
        });
        metrics::gauge!("scryer_download_queue_snapshot_items").set(item_count as f64);
        metrics::counter!("scryer_download_queue_snapshot_refresh_total", "result" => "success")
            .increment(1);
        self.schedule_commit();
    }

    pub async fn stage_upserts(&self, items: Vec<DownloadQueueItem>) {
        if items.is_empty() {
            return;
        }
        let mut pending = self.pending.lock().await;
        if pending.is_none() {
            let (items, positions) =
                index_download_queue_items(self.state.read().await.items.to_vec());
            *pending = Some(PendingDownloadQueueSnapshot {
                items,
                positions,
                updated_at: Utc::now(),
                clear_refresh_error: false,
            });
        }
        let snapshot = pending.as_mut().expect("pending snapshot initialized");
        for item in items {
            let key = download_queue_cache_identity(&item);
            if let Some(index) = snapshot.positions.get(&key).copied() {
                snapshot.items[index] = item;
            } else {
                snapshot.positions.insert(key, snapshot.items.len());
                snapshot.items.push(item);
            }
        }
        snapshot.updated_at = Utc::now();
        drop(pending);
        self.schedule_commit();
    }

    pub async fn stage_import_transfer_progress(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        phase: scryer_domain::ImportTransferPhase,
        bytes: i64,
        total_bytes: i64,
    ) {
        let mut pending = self.pending.lock().await;
        if pending.is_none() {
            let (items, positions) =
                index_download_queue_items(self.state.read().await.items.to_vec());
            *pending = Some(PendingDownloadQueueSnapshot {
                items,
                positions,
                updated_at: Utc::now(),
                clear_refresh_error: false,
            });
        }
        let snapshot = pending.as_mut().expect("pending snapshot initialized");
        let Some(item) = snapshot.items.iter_mut().find(|item| {
            item.download_client_item_id == download_client_item_id
                && client_id.map_or_else(
                    || item.client_type.eq_ignore_ascii_case(client_type),
                    |client_id| item.client_id.eq_ignore_ascii_case(client_id),
                )
        }) else {
            return;
        };
        if item.import_transfer_phase == Some(phase)
            && item.import_transfer_bytes == Some(bytes)
            && item.import_transfer_total_bytes == Some(total_bytes)
        {
            return;
        }
        let updated_at = Utc::now().to_rfc3339();
        item.import_transfer_phase = Some(phase);
        item.import_transfer_bytes = Some(bytes);
        item.import_transfer_total_bytes = Some(total_bytes);
        item.import_transfer_started_at
            .get_or_insert_with(|| updated_at.clone());
        item.import_transfer_updated_at = Some(updated_at);
        snapshot.updated_at = Utc::now();
        drop(pending);
        self.schedule_commit();
    }

    pub async fn mark_refresh_failed(&self, error: impl Into<String>) {
        let error = error.into();
        let mut state = self.state.write().await;
        let changed = state.refresh_error.as_deref() != Some(error.as_str());
        if !changed {
            drop(state);
            metrics::counter!("scryer_download_queue_snapshot_refresh_total", "result" => "error")
                .increment(1);
            return;
        }
        state.revision = state.revision.saturating_add(1);
        state.refresh_error = Some(Arc::from(error));
        let sync = DownloadQueueSync {
            revision: state.revision,
            updated_at: state.updated_at,
        };
        drop(state);
        self.sync_tx.send_replace(sync);
        metrics::counter!("scryer_download_queue_snapshot_refresh_total", "result" => "error")
            .increment(1);
    }

    fn schedule_commit(&self) {
        use std::sync::atomic::Ordering;

        if self
            .commit_scheduled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let cache = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(DOWNLOAD_QUEUE_SNAPSHOT_COALESCE_WINDOW).await;
                cache.commit_pending().await;
                cache.commit_scheduled.store(false, Ordering::Release);

                if cache.pending.lock().await.is_none()
                    || cache
                        .commit_scheduled
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                {
                    break;
                }
            }
        });
    }

    async fn commit_pending(&self) {
        let Some(pending) = self.pending.lock().await.take() else {
            return;
        };
        let mut state = self.state.write().await;
        let items_changed = state.items.as_ref() != pending.items.as_slice();
        let readiness_changed = !state.ready;
        let error_changed = pending.clear_refresh_error && state.refresh_error.is_some();
        let changed = items_changed || readiness_changed || error_changed;
        if items_changed {
            state.items = Arc::from(pending.items);
        }
        state.updated_at = Some(pending.updated_at);
        state.ready = true;
        if pending.clear_refresh_error {
            state.refresh_error = None;
        }
        if !changed {
            return;
        }
        state.revision = state.revision.saturating_add(1);
        let sync = DownloadQueueSync {
            revision: state.revision,
            updated_at: state.updated_at,
        };
        drop(state);
        self.sync_tx.send_replace(sync);
        metrics::counter!("scryer_download_queue_revision_notifications_total").increment(1);
    }
}

fn download_queue_cache_identity(item: &DownloadQueueItem) -> (String, String) {
    let client = if item.client_id.trim().is_empty() {
        item.client_type.trim()
    } else {
        item.client_id.trim()
    };
    (
        client.to_ascii_lowercase(),
        item.download_client_item_id.clone(),
    )
}

fn index_download_queue_items(
    items: Vec<DownloadQueueItem>,
) -> (Vec<DownloadQueueItem>, HashMap<(String, String), usize>) {
    let mut deduped = Vec::with_capacity(items.len());
    let mut positions = HashMap::with_capacity(items.len());
    for item in items {
        let key = download_queue_cache_identity(&item);
        if let Some(index) = positions.get(&key).copied() {
            deduped[index] = item;
        } else {
            positions.insert(key, deduped.len());
            deduped.push(item);
        }
    }
    (deduped, positions)
}

#[cfg(test)]
mod download_queue_snapshot_cache_tests {
    use super::*;

    fn item(index: usize) -> DownloadQueueItem {
        let id = format!("item-{index}");
        DownloadQueueItem {
            id: id.clone(),
            title_id: None,
            episode_id: None,
            title_name: format!("Episode {index}"),
            facet: None,
            category: None,
            client_id: "client-1".to_string(),
            client_name: "qBittorrent".to_string(),
            client_type: "qbittorrent".to_string(),
            state: DownloadQueueState::Downloading,
            progress_percent: (index % 100) as u8,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: Some(index as i64),
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: id,
            download_id: None,
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            is_scryer_origin: true,
            source_provider: None,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
            seeding: None,
        }
    }

    #[tokio::test]
    async fn cold_cache_is_not_ready_and_is_stale() {
        let snapshot = DownloadQueueSnapshotCache::default().snapshot().await;
        assert!(!snapshot.ready);
        assert!(snapshot.stale_at(Utc::now()));
        assert_eq!(snapshot.revision, 0);
        assert!(snapshot.items.is_empty());
    }

    #[tokio::test]
    async fn cache_handles_zero_to_three_thousand_items() {
        for count in [0, 100, 1_000, 3_000] {
            let cache = DownloadQueueSnapshotCache::default();
            cache
                .stage_success((0..count).map(item).collect::<Vec<_>>())
                .await;
            cache.commit_pending().await;
            let snapshot = cache.snapshot().await;
            assert_eq!(snapshot.items.len(), count);
            assert_eq!(snapshot.revision, 1);
            assert!(snapshot.ready);
            assert!(!snapshot.stale_at(Utc::now()));
        }
    }

    #[tokio::test]
    async fn one_thousand_item_burst_commits_one_revision_and_deduplicates() {
        let cache = DownloadQueueSnapshotCache::default();
        for index in 0..1_000 {
            cache.stage_upserts(vec![item(index)]).await;
        }
        cache.stage_upserts(vec![item(999)]).await;
        tokio::time::sleep(
            DOWNLOAD_QUEUE_SNAPSHOT_COALESCE_WINDOW + std::time::Duration::from_millis(50),
        )
        .await;

        let snapshot = cache.snapshot().await;
        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.items.len(), 1_000);
    }

    #[tokio::test]
    async fn failed_refresh_retains_last_successful_snapshot_and_marks_it_stale() {
        let cache = DownloadQueueSnapshotCache::default();
        cache.stage_success((0..100).map(item).collect()).await;
        cache.commit_pending().await;
        cache.mark_refresh_failed("client unavailable").await;
        cache
            .stage_partial_success((0..50).map(item).collect())
            .await;
        cache.commit_pending().await;

        let snapshot = cache.snapshot().await;
        assert_eq!(snapshot.items.len(), 50);
        assert_eq!(snapshot.revision, 3);
        assert!(snapshot.ready);
        assert!(snapshot.stale_at(Utc::now()));
        assert_eq!(
            snapshot.refresh_error.as_deref(),
            Some("client unavailable")
        );
    }

    #[tokio::test]
    async fn identical_polls_and_failures_do_not_advance_revision_but_recovery_does() {
        let cache = DownloadQueueSnapshotCache::default();
        let items = (0..100).map(item).collect::<Vec<_>>();
        cache.stage_success(items.clone()).await;
        cache.commit_pending().await;
        assert_eq!(cache.snapshot().await.revision, 1);

        cache.stage_success(items.clone()).await;
        cache.commit_pending().await;
        assert_eq!(cache.snapshot().await.revision, 1);

        cache.mark_refresh_failed("client unavailable").await;
        assert_eq!(cache.snapshot().await.revision, 2);
        cache.mark_refresh_failed("client unavailable").await;
        assert_eq!(cache.snapshot().await.revision, 2);

        cache.stage_success(items).await;
        cache.commit_pending().await;
        let recovered = cache.snapshot().await;
        assert_eq!(recovered.revision, 3);
        assert!(recovered.refresh_error.is_none());
        assert!(!recovered.stale_at(Utc::now()));
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CachedWantedProjection {
    pub generation: u64,
    pub time_bucket: Option<i64>,
    pub rows: Arc<[crate::acquisition::wanted_views::WantedScopeView]>,
}

#[derive(Clone)]
pub struct AppRuntimeAcquisitionState {
    pub acquisition_wake: Arc<tokio::sync::Notify>,
    pub download_submission_guards: DownloadSubmissionGuardTable,
    pub download_failure_guards: DownloadFailureGuardTable,
    pub(crate) release_candidate_passwords:
        Arc<std::sync::Mutex<HashMap<String, ReleaseCandidatePasswordTicket>>>,
    pub rss_seen_guids: Arc<tokio::sync::RwLock<HashSet<String>>>,
    pub tracked_download_handle: Option<tracked_downloads::TrackedDownloadHandle>,
    pub tracked_download_snapshot:
        Arc<tokio::sync::RwLock<HashMap<String, tracked_downloads::TrackedDownloadQueueMetadata>>>,
    pub download_queue_snapshot: DownloadQueueSnapshotCache,
    pub(crate) download_queue_read_model: DownloadQueueReadModelCache,
    pub(crate) wanted_projection_generation: Arc<std::sync::atomic::AtomicU64>,
    pub(crate) wanted_projection_cache:
        Arc<tokio::sync::RwLock<HashMap<crate::types::WantedKind, CachedWantedProjection>>>,
    pub(crate) wanted_projection_build_lock: Arc<tokio::sync::Mutex<()>>,
    pub(crate) download_client_category_admission:
        Arc<tokio::sync::RwLock<Option<Arc<DownloadClientCategoryAdmissionSnapshot>>>>,
    /// Cancellation tokens for in-flight interactive acquisition-search jobs
    ///, keyed by job-run id — mirrors the library-scan cancel map.
    pub acquisition_search_cancellation_tokens:
        Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
    /// In-memory registry of interactive release-search jobs (hotfix 0.17.1),
    /// keyed by job id. Ephemeral by design — see
    /// `catalog::interactive_release_search` for the eviction rules.
    pub(crate) interactive_release_searches: Arc<
        Mutex<
            HashMap<
                String,
                crate::catalog::interactive_release_search::InteractiveReleaseSearchJobEntry,
            >,
        >,
    >,
}

impl AppRuntimeAcquisitionState {
    pub(crate) fn invalidate_wanted_projection_cache(&self) {
        self.wanted_projection_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
}

#[derive(Clone, Default)]
pub(crate) struct DownloadClientCategoryAdmissionSnapshot {
    pub(crate) default_categories: HashSet<String>,
    pub(crate) categories_by_client: HashMap<String, HashSet<String>>,
}

pub(crate) fn normalize_download_client_category(category: &str) -> String {
    category.trim().to_ascii_lowercase()
}

impl DownloadClientCategoryAdmissionSnapshot {
    pub(crate) fn knows_category(&self, category: &str) -> bool {
        let category = normalize_download_client_category(category);
        !category.is_empty()
            && (self.default_categories.contains(&category)
                || self
                    .categories_by_client
                    .values()
                    .any(|categories| categories.contains(&category)))
    }
}

pub(crate) fn download_observation_is_admitted(
    has_scryer_submission: bool,
    category: Option<&str>,
    snapshot: Option<&DownloadClientCategoryAdmissionSnapshot>,
) -> bool {
    has_scryer_submission
        || category
            .and_then(|category| {
                let category = category.trim();
                (!category.is_empty()).then_some(category)
            })
            .zip(snapshot)
            .is_some_and(|(category, snapshot)| snapshot.knows_category(category))
}

/// Whether a completed download declares itself another manager's work: the
/// `drone` parameter Sonarr/Radarr stamp on their own grabs.
pub(crate) fn completed_download_claims_external_manager(
    completed: &scryer_domain::CompletedDownload,
) -> bool {
    completed
        .parameters
        .iter()
        .any(|(key, _)| key.trim().eq_ignore_ascii_case("drone"))
}

/// The one admission decision for a completed download entering any Scryer
/// surface (automatic import, manual-import source lookup, retained manual
/// source), so the tracked check, the manual-import resolvers, and the queue
/// filter cannot disagree about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CompletedDownloadAdmission {
    Admitted,
    /// Claimed by another manager (`drone` parameter) and not a Scryer grab.
    ExternalManager,
    /// A downloader observation whose category Scryer never configured (or
    /// blank), or admission is not ready yet.
    NotAdmitted {
        category: Option<String>,
        admission_snapshot_missing: bool,
    },
}

impl AppUseCase {
    /// `fallback_category` is the queue item's category when the completed
    /// history entry carries none.
    pub(crate) async fn completed_download_admission(
        &self,
        has_scryer_submission: bool,
        completed: &scryer_domain::CompletedDownload,
        fallback_category: Option<&str>,
    ) -> CompletedDownloadAdmission {
        if has_scryer_submission {
            return CompletedDownloadAdmission::Admitted;
        }
        if completed_download_claims_external_manager(completed) {
            return CompletedDownloadAdmission::ExternalManager;
        }
        let category = completed
            .category
            .as_deref()
            .or(fallback_category)
            .map(str::trim)
            .filter(|category| !category.is_empty());
        let snapshot = self.download_client_category_admission_snapshot().await;
        if download_observation_is_admitted(false, category, snapshot.as_deref()) {
            CompletedDownloadAdmission::Admitted
        } else {
            CompletedDownloadAdmission::NotAdmitted {
                category: category.map(str::to_string),
                admission_snapshot_missing: snapshot.is_none(),
            }
        }
    }
}

#[cfg(test)]
mod download_client_category_admission_tests {
    use super::*;

    fn snapshot() -> DownloadClientCategoryAdmissionSnapshot {
        DownloadClientCategoryAdmissionSnapshot {
            default_categories: HashSet::from(["movie".to_string()]),
            categories_by_client: HashMap::from([(
                "client-2".to_string(),
                HashSet::from(["series".to_string()]),
            )]),
        }
    }

    #[test]
    fn tracked_submissions_bypass_category_admission() {
        assert!(download_observation_is_admitted(true, None, None));
        assert!(download_observation_is_admitted(
            true,
            Some("unknown"),
            Some(&snapshot())
        ));
    }

    #[test]
    fn observations_require_a_normalized_known_category() {
        let snapshot = snapshot();
        assert!(download_observation_is_admitted(
            false,
            Some("  MoViE "),
            Some(&snapshot)
        ));
        assert!(download_observation_is_admitted(
            false,
            Some("SERIES"),
            Some(&snapshot)
        ));
        assert!(!download_observation_is_admitted(
            false,
            Some("unknown"),
            Some(&snapshot)
        ));
        assert!(!download_observation_is_admitted(
            false,
            None,
            Some(&snapshot)
        ));
        assert!(!download_observation_is_admitted(
            false,
            Some("movie"),
            None
        ));
    }
}

pub(crate) struct ReleaseCandidatePasswordTicket {
    pub actor_id: String,
    pub title_id: String,
    pub scope_kind: String,
    pub scope_id: Option<String>,
    pub source_hint: String,
    pub source_title: String,
    pub password: String,
    pub expires_at: DateTime<Utc>,
}

const MAX_CONCURRENT_ARCHIVE_EXTRACTIONS: usize = 1;

#[derive(Clone)]
pub(crate) struct ImportExecutionCoordinator {
    permit: Arc<tokio::sync::Mutex<()>>,
    archive_extraction_permit: Arc<tokio::sync::Semaphore>,
}

impl Default for ImportExecutionCoordinator {
    fn default() -> Self {
        Self {
            permit: Arc::new(tokio::sync::Mutex::new(())),
            archive_extraction_permit: Arc::new(tokio::sync::Semaphore::new(
                MAX_CONCURRENT_ARCHIVE_EXTRACTIONS,
            )),
        }
    }
}

impl ImportExecutionCoordinator {
    pub(crate) async fn acquire(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.permit.clone().lock_owned().await
    }

    pub(crate) async fn acquire_archive_extraction(&self) -> tokio::sync::OwnedSemaphorePermit {
        self.archive_extraction_permit
            .clone()
            .acquire_owned()
            .await
            .expect("archive extraction semaphore is never closed")
    }
}

#[cfg(test)]
mod import_execution_coordinator_tests {
    use super::ImportExecutionCoordinator;
    use std::time::Duration;

    #[tokio::test]
    async fn permits_only_one_import_execution_at_a_time() {
        let coordinator = ImportExecutionCoordinator::default();
        let first = coordinator.acquire().await;
        let waiting = tokio::spawn({
            let coordinator = coordinator.clone();
            async move {
                let _second = coordinator.acquire().await;
            }
        });

        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());

        drop(first);
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("second import should acquire after the first completes")
            .expect("waiting import task should complete");
    }

    #[tokio::test]
    async fn permits_only_one_archive_extraction_at_a_time() {
        let coordinator = ImportExecutionCoordinator::default();
        let first = coordinator.acquire_archive_extraction().await;
        let waiting = tokio::spawn({
            let coordinator = coordinator.clone();
            async move {
                let _second = coordinator.acquire_archive_extraction().await;
            }
        });

        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());

        drop(first);
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("second extraction should acquire after the first completes")
            .expect("waiting extraction task should complete");
    }
}

#[derive(Clone)]
pub struct AppRuntimeImportState {
    pub(crate) execution_coordinator: ImportExecutionCoordinator,
    pub external_import_warmup_orchestrator: ExternalImportMonitorWarmupOrchestrator,
    pub external_import_apply_lock: Arc<tokio::sync::Mutex<()>>,
    pub external_import_source_chunk_cleanup_done: Arc<tokio::sync::Mutex<bool>>,
    pub(crate) same_path_upgrade_guard_lock: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Clone)]
pub struct AppRuntimeLibraryState {
    pub library_scan_tracker: LibraryScanTracker,
    pub library_scan_cancellation_tokens:
        Arc<Mutex<HashMap<String, tokio_util::sync::CancellationToken>>>,
    pub library_scan_title_walk_limit: Arc<Semaphore>,
    pub library_scan_analysis_limit: Arc<Semaphore>,
}

#[derive(Clone)]
pub struct AppRuntimeJobState {
    pub job_run_tracker: JobRunTracker,
    pub discovery_sync_wake: Arc<tokio::sync::Notify>,
    pub backup_execution_guards: BackupExecutionGuardTable,
    pub interactive_operation_guards: InteractiveOperationGuardTable,
    pub title_deletion_lock: Arc<tokio::sync::Mutex<()>>,
    /// Single-flight guard for the interactive acquisition-search job — mirrors `title_deletion_lock`.
    pub acquisition_search_lock: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Clone)]
pub struct AppRuntimeHealthState {
    pub results: Arc<tokio::sync::RwLock<Vec<HealthCheckResult>>>,
}

#[derive(Clone)]
pub struct AppRuntimePluginState {
    pub plugin_operation_guards: PluginOperationGuardTable,
    pub plugin_install_orchestrator: PluginInstallOrchestrator,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimePerformanceClass {
    Slow,
    Fast,
}

impl std::fmt::Display for RuntimePerformanceClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Slow => f.write_str("slow"),
            Self::Fast => f.write_str("fast"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimePerformanceSnapshot {
    pub cpu_class: RuntimePerformanceClass,
    pub config_io_class: RuntimePerformanceClass,
    pub cpu_probe_elapsed_ms: Option<u64>,
    pub config_io_probe_elapsed_ms: Option<u64>,
}

impl RuntimePerformanceSnapshot {
    pub fn slow() -> Self {
        Self {
            cpu_class: RuntimePerformanceClass::Slow,
            config_io_class: RuntimePerformanceClass::Slow,
            cpu_probe_elapsed_ms: None,
            config_io_probe_elapsed_ms: None,
        }
    }
}

#[derive(Clone)]
pub struct AppRuntimeEnvironmentState {
    pub build_lane: BinaryLane,
    pub build_class: BinaryClass,
    pub(crate) supported_plugin_required_features: Arc<HashSet<String>>,
    pub(crate) config_dir: Arc<PathBuf>,
    pub(crate) performance_snapshot: Arc<OnceCell<RuntimePerformanceSnapshot>>,
    fixed_now: Arc<std::sync::RwLock<Option<DateTime<Utc>>>>,
}

impl AppRuntimeEnvironmentState {
    pub fn new<I, S>(
        build_lane: BinaryLane,
        config_dir: impl Into<PathBuf>,
        supported_plugin_required_features: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            build_lane,
            build_class: build_lane.binary_class(),
            supported_plugin_required_features: normalize_supported_plugin_required_features(
                supported_plugin_required_features,
            ),
            config_dir: Arc::new(config_dir.into()),
            performance_snapshot: Arc::new(OnceCell::new()),
            fixed_now: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    pub(crate) fn now(&self) -> DateTime<Utc> {
        self.fixed_now
            .read()
            .ok()
            .and_then(|guard| *guard)
            .unwrap_or_else(Utc::now)
    }

    pub fn set_fixed_now_for_tests(&self, now: Option<DateTime<Utc>>) {
        if let Ok(mut guard) = self.fixed_now.write() {
            *guard = now;
        }
    }
}

pub(super) fn normalize_supported_plugin_required_features<I, S>(
    features: I,
) -> Arc<HashSet<String>>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    Arc::new(
        features
            .into_iter()
            .map(Into::into)
            .map(|feature| feature.trim().to_ascii_lowercase())
            .filter(|feature| !feature.is_empty())
            .collect::<HashSet<_>>(),
    )
}

#[derive(Clone)]
pub struct AppRuntimeIntegrationState {
    pub managed_indexer_sync_lock: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Clone)]
pub struct AppRuntimeSecurityState {
    pub(super) recovery_admin_login_enabled: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone)]
pub struct AppRuntimeState {
    pub environment: AppRuntimeEnvironmentState,
    pub security: AppRuntimeSecurityState,
    pub events: AppRuntimeEventState,
    pub catalog: AppRuntimeCatalogState,
    pub acquisition: AppRuntimeAcquisitionState,
    pub imports: AppRuntimeImportState,
    pub library: AppRuntimeLibraryState,
    pub jobs: AppRuntimeJobState,
    pub health: AppRuntimeHealthState,
    pub plugins: AppRuntimePluginState,
    pub integrations: AppRuntimeIntegrationState,
}

impl AppRuntimeState {
    pub fn new<I, S>(
        build_lane: BinaryLane,
        config_dir: impl Into<PathBuf>,
        supported_plugin_required_features: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let (domain_event_tx, _domain_event_rx) = broadcast::channel(256);
        // Match the main domain-event buffer so short notification bursts can queue wake hints
        // while the dispatcher catches up from persisted offsets.
        let (notification_event_tx, _notification_event_rx) = broadcast::channel(256);
        let (import_history_tx, _) = broadcast::channel::<()>(16);
        let (indexers_changed_tx, _) = broadcast::channel::<()>(16);
        let (provider_catalog_changed_tx, _) = broadcast::channel::<Vec<ProviderCatalogFamily>>(16);
        let (settings_changed_tx, _) = broadcast::channel::<Vec<String>>(16);

        Self {
            environment: AppRuntimeEnvironmentState::new(
                build_lane,
                config_dir,
                supported_plugin_required_features,
            ),
            security: AppRuntimeSecurityState {
                recovery_admin_login_enabled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
            events: AppRuntimeEventState {
                domain_event_broadcast: domain_event_tx,
                notification_event_broadcast: notification_event_tx,
                import_history_broadcast: import_history_tx,
                indexers_changed_broadcast: indexers_changed_tx,
                provider_catalog_changed_broadcast: provider_catalog_changed_tx,
                settings_changed_broadcast: settings_changed_tx,
            },
            catalog: AppRuntimeCatalogState {
                quality_profile_reference_lock: Arc::new(tokio::sync::Mutex::new(())),
                monitored_title_matcher: Arc::new(RwLock::new(
                    crate::import_title_resolution::MonitoredTitleMatcherCache::default(),
                )),
                poster_wake: Arc::new(tokio::sync::Notify::new()),
                fanart_wake: Arc::new(tokio::sync::Notify::new()),
                title_hydration_wake: Arc::new(tokio::sync::Notify::new()),
                title_recommendation_refresh_queue: Arc::new(tokio::sync::Mutex::new(
                    crate::catalog_workflow::TitleRecommendationRefreshQueue::default(),
                )),
                title_recommendation_refresh_wake: Arc::new(tokio::sync::Notify::new()),
                image_processing_limit: Arc::new(Semaphore::new(4)),
                title_image_maintenance_lock: Arc::new(tokio::sync::RwLock::new(())),
                title_image_cache_clear_scheduled: Arc::new(std::sync::atomic::AtomicBool::new(
                    false,
                )),
            },
            acquisition: AppRuntimeAcquisitionState {
                acquisition_wake: Arc::new(tokio::sync::Notify::new()),
                download_submission_guards: DownloadSubmissionGuardTable::default(),
                download_failure_guards: DownloadFailureGuardTable::default(),
                release_candidate_passwords: Arc::new(std::sync::Mutex::new(HashMap::new())),
                rss_seen_guids: Arc::new(tokio::sync::RwLock::new(HashSet::new())),
                tracked_download_handle: None,
                tracked_download_snapshot: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
                download_queue_snapshot: DownloadQueueSnapshotCache::default(),
                download_queue_read_model: DownloadQueueReadModelCache::default(),
                wanted_projection_generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
                wanted_projection_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
                wanted_projection_build_lock: Arc::new(tokio::sync::Mutex::new(())),
                download_client_category_admission: Arc::new(tokio::sync::RwLock::new(None)),
                acquisition_search_cancellation_tokens: Arc::new(Mutex::new(HashMap::new())),
                interactive_release_searches: Arc::new(Mutex::new(HashMap::new())),
            },
            imports: AppRuntimeImportState {
                execution_coordinator: ImportExecutionCoordinator::default(),
                external_import_warmup_orchestrator:
                    ExternalImportMonitorWarmupOrchestrator::default(),
                external_import_apply_lock: Arc::new(tokio::sync::Mutex::new(())),
                external_import_source_chunk_cleanup_done: Arc::new(tokio::sync::Mutex::new(false)),
                same_path_upgrade_guard_lock: Arc::new(tokio::sync::Mutex::new(())),
            },
            library: AppRuntimeLibraryState {
                library_scan_tracker: LibraryScanTracker::new(),
                library_scan_cancellation_tokens: Arc::new(Mutex::new(HashMap::new())),
                library_scan_title_walk_limit: Arc::new(Semaphore::new(
                    LIBRARY_SCAN_GLOBAL_TITLE_WALK_CONCURRENCY,
                )),
                library_scan_analysis_limit: Arc::new(Semaphore::new(
                    GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY,
                )),
            },
            jobs: AppRuntimeJobState {
                job_run_tracker: JobRunTracker::new(),
                discovery_sync_wake: Arc::new(tokio::sync::Notify::new()),
                backup_execution_guards: BackupExecutionGuardTable::default(),
                interactive_operation_guards: InteractiveOperationGuardTable::default(),
                title_deletion_lock: Arc::new(tokio::sync::Mutex::new(())),
                acquisition_search_lock: Arc::new(tokio::sync::Mutex::new(())),
            },
            health: AppRuntimeHealthState {
                results: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            },
            plugins: AppRuntimePluginState {
                plugin_operation_guards: PluginOperationGuardTable::default(),
                plugin_install_orchestrator: PluginInstallOrchestrator::default(),
            },
            integrations: AppRuntimeIntegrationState {
                managed_indexer_sync_lock: Arc::new(tokio::sync::Mutex::new(())),
            },
        }
    }
}

impl Default for AppRuntimeState {
    fn default() -> Self {
        Self::new(
            BinaryLane::Portable,
            PathBuf::from("."),
            Vec::<String>::new(),
        )
    }
}
