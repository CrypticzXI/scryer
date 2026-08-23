use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalImportMonitorWarmupStatus {
    Queued,
    Running,
    Completed,
    Canceled,
    Failed,
}

impl ExternalImportMonitorWarmupStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Canceled | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalImportMonitorWarmupPhase {
    LoadingIndexers,
    LoadingMovies,
    LoadingSeries,
    LoadingEpisodes,
    BuildingSnapshot,
    Ready,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExternalImportMonitorWarmupPhaseProgress {
    pub total: i32,
    pub completed: i32,
    pub failed: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalImportMonitorWarmupProgressSnapshot {
    pub session_id: String,
    pub status: ExternalImportMonitorWarmupStatus,
    pub phase: ExternalImportMonitorWarmupPhase,
    pub started_at: String,
    pub updated_at: String,
    pub overall_total_known: bool,
    pub overall_progress: ExternalImportMonitorWarmupPhaseProgress,
    pub movies_total_known: bool,
    pub movies_progress: ExternalImportMonitorWarmupPhaseProgress,
    pub series_total_known: bool,
    pub series_progress: ExternalImportMonitorWarmupPhaseProgress,
    pub episode_fetch_total_known: bool,
    pub episode_fetch_expected_total: Option<i32>,
    pub episode_fetch_expected_monitored_total: Option<i32>,
    pub episode_fetch_progress: ExternalImportMonitorWarmupPhaseProgress,
    pub snapshot_build_total_known: bool,
    pub snapshot_build_progress: ExternalImportMonitorWarmupPhaseProgress,
    pub matched_movie_count: i32,
    pub matched_series_count: i32,
    pub unmatched_movie_count: i32,
    pub unmatched_series_count: i32,
    pub ambiguous_movie_count: i32,
    pub ambiguous_series_count: i32,
    pub error_message: Option<String>,
}

impl ExternalImportMonitorWarmupProgressSnapshot {
    pub fn new(session_id: String) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            session_id,
            status: ExternalImportMonitorWarmupStatus::Queued,
            phase: ExternalImportMonitorWarmupPhase::LoadingMovies,
            started_at: now.clone(),
            updated_at: now,
            overall_total_known: false,
            overall_progress: ExternalImportMonitorWarmupPhaseProgress::default(),
            movies_total_known: false,
            movies_progress: ExternalImportMonitorWarmupPhaseProgress::default(),
            series_total_known: false,
            series_progress: ExternalImportMonitorWarmupPhaseProgress::default(),
            episode_fetch_total_known: false,
            episode_fetch_expected_total: None,
            episode_fetch_expected_monitored_total: None,
            episode_fetch_progress: ExternalImportMonitorWarmupPhaseProgress::default(),
            snapshot_build_total_known: false,
            snapshot_build_progress: ExternalImportMonitorWarmupPhaseProgress::default(),
            matched_movie_count: 0,
            matched_series_count: 0,
            unmatched_movie_count: 0,
            unmatched_series_count: 0,
            ambiguous_movie_count: 0,
            ambiguous_series_count: 0,
            error_message: None,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = Utc::now().to_rfc3339();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ExternalImportArrSourceKind {
    Sonarr,
    Radarr,
}

impl ExternalImportArrSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sonarr => "sonarr",
            Self::Radarr => "radarr",
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ExternalImportArrSourceSeriesEntry {
    pub series: crate::external_import::ArrSeries,
    pub episodes: Vec<crate::external_import::ArrEpisode>,
}

#[derive(Clone, Debug)]
pub struct ExternalImportArrSourceWarmupResult {
    pub source_key: String,
    pub kind: ExternalImportArrSourceKind,
    pub base_url: String,
    pub version: Option<String>,
    pub root_folders: Vec<crate::external_import::ArrRootFolder>,
    pub title_root_paths: Vec<String>,
    pub naming_config: Option<crate::external_import::ArrNamingConfig>,
    pub media_management_config: Option<crate::external_import::ArrMediaManagementConfig>,
    pub metadata_providers: Vec<crate::external_import::ArrMetadataProvider>,
    pub quality_profiles: Vec<crate::external_import::ArrQualityProfile>,
    pub signal_warnings: Vec<String>,
    pub download_clients: Vec<crate::external_import::ArrDownloadClient>,
    pub indexers: Vec<crate::external_import::ArrIndexer>,
}

#[derive(Clone, Debug)]
pub struct ExternalImportProwlarrWarmupResult {
    pub base_url: String,
    /// The operator-entered Prowlarr API key the discovery ran with. Preview
    /// merges it into the import group so downstream consumers see the real
    /// credential, never a placeholder.
    pub api_key: String,
    pub version: Option<String>,
    pub plan: crate::IndexerSyncPlan,
}

#[derive(Clone)]
pub struct ExternalImportMonitorWarmupBeginResult {
    pub snapshot: ExternalImportMonitorWarmupProgressSnapshot,
    pub created: bool,
    pub cancel_token: tokio_util::sync::CancellationToken,
    pub replaced_session_id: Option<String>,
}

#[derive(Clone)]
struct ExternalImportMonitorWarmupSessionHandle {
    actor_user_id: String,
    connection_fingerprint: String,
    claimed: bool,
    cancel_token: tokio_util::sync::CancellationToken,
    tx: tokio::sync::watch::Sender<ExternalImportMonitorWarmupProgressSnapshot>,
    scan_hints: Option<crate::LibraryScanHintSet>,
    arr_source_result: Option<ExternalImportArrSourceWarmupResult>,
    prowlarr_result: Option<ExternalImportProwlarrWarmupResult>,
}

#[derive(Default)]
struct ExternalImportMonitorWarmupOrchestratorState {
    session_ids_by_actor_fingerprint: HashMap<(String, String), String>,
    sessions_by_id: HashMap<String, ExternalImportMonitorWarmupSessionHandle>,
}

#[derive(Clone, Default)]
pub struct ExternalImportMonitorWarmupOrchestrator {
    state: Arc<tokio::sync::Mutex<ExternalImportMonitorWarmupOrchestratorState>>,
}

impl ExternalImportMonitorWarmupOrchestrator {
    pub async fn begin(
        &self,
        actor_user_id: &str,
        connection_fingerprint: &str,
        initial_snapshot: ExternalImportMonitorWarmupProgressSnapshot,
    ) -> ExternalImportMonitorWarmupBeginResult {
        let actor_key = (
            actor_user_id.to_string(),
            connection_fingerprint.to_string(),
        );
        let mut state = self.state.lock().await;
        let mut replaced_session_id = None;

        if let Some(existing_session_id) = state
            .session_ids_by_actor_fingerprint
            .get(&actor_key)
            .cloned()
        {
            if let Some(existing_handle) = state.sessions_by_id.get(&existing_session_id) {
                let existing_snapshot = existing_handle.tx.borrow().clone();
                if matches!(
                    existing_snapshot.status,
                    ExternalImportMonitorWarmupStatus::Queued
                        | ExternalImportMonitorWarmupStatus::Running
                ) {
                    return ExternalImportMonitorWarmupBeginResult {
                        snapshot: existing_snapshot,
                        created: false,
                        cancel_token: existing_handle.cancel_token.clone(),
                        replaced_session_id: None,
                    };
                }
            }

            state.session_ids_by_actor_fingerprint.remove(&actor_key);
            state.sessions_by_id.remove(&existing_session_id);
            replaced_session_id = Some(existing_session_id);
        }

        let cancel_token = tokio_util::sync::CancellationToken::new();
        let (tx, _rx) = tokio::sync::watch::channel(initial_snapshot.clone());
        state
            .session_ids_by_actor_fingerprint
            .insert(actor_key, initial_snapshot.session_id.clone());
        state.sessions_by_id.insert(
            initial_snapshot.session_id.clone(),
            ExternalImportMonitorWarmupSessionHandle {
                actor_user_id: actor_user_id.to_string(),
                connection_fingerprint: connection_fingerprint.to_string(),
                claimed: false,
                cancel_token: cancel_token.clone(),
                tx,
                scan_hints: None,
                arr_source_result: None,
                prowlarr_result: None,
            },
        );

        ExternalImportMonitorWarmupBeginResult {
            snapshot: initial_snapshot,
            created: true,
            cancel_token,
            replaced_session_id,
        }
    }

    pub async fn subscribe(
        &self,
        actor_user_id: &str,
        session_id: &str,
    ) -> Option<tokio::sync::watch::Receiver<ExternalImportMonitorWarmupProgressSnapshot>> {
        let state = self.state.lock().await;
        state.sessions_by_id.get(session_id).and_then(|handle| {
            (handle.actor_user_id == actor_user_id).then(|| handle.tx.subscribe())
        })
    }

    pub async fn snapshot(
        &self,
        actor_user_id: &str,
        session_id: &str,
    ) -> Option<ExternalImportMonitorWarmupProgressSnapshot> {
        let state = self.state.lock().await;
        state.sessions_by_id.get(session_id).and_then(|handle| {
            (handle.actor_user_id == actor_user_id).then(|| handle.tx.borrow().clone())
        })
    }

    pub async fn update(
        &self,
        session_id: &str,
        snapshot: ExternalImportMonitorWarmupProgressSnapshot,
    ) -> bool {
        let state = self.state.lock().await;
        let Some(handle) = state.sessions_by_id.get(session_id) else {
            return false;
        };
        handle.tx.send_replace(snapshot);
        true
    }

    pub async fn set_scan_hints(
        &self,
        actor_user_id: &str,
        session_id: &str,
        scan_hints: crate::LibraryScanHintSet,
    ) -> bool {
        let mut state = self.state.lock().await;
        if !state.sessions_by_id.contains_key(session_id) {
            if scan_hints.is_empty() {
                return false;
            }
            let mut snapshot =
                ExternalImportMonitorWarmupProgressSnapshot::new(session_id.to_string());
            snapshot.status = ExternalImportMonitorWarmupStatus::Completed;
            snapshot.phase = ExternalImportMonitorWarmupPhase::Ready;
            let (tx, _rx) = tokio::sync::watch::channel(snapshot);
            state.sessions_by_id.insert(
                session_id.to_string(),
                ExternalImportMonitorWarmupSessionHandle {
                    actor_user_id: actor_user_id.to_string(),
                    connection_fingerprint: session_id.to_string(),
                    claimed: true,
                    cancel_token: tokio_util::sync::CancellationToken::new(),
                    tx,
                    scan_hints: None,
                    arr_source_result: None,
                    prowlarr_result: None,
                },
            );
        }
        let Some(handle) = state.sessions_by_id.get_mut(session_id) else {
            return false;
        };
        if handle.actor_user_id != actor_user_id {
            return false;
        }
        handle.scan_hints = (!scan_hints.is_empty()).then_some(scan_hints);
        true
    }

    pub async fn scan_hints(
        &self,
        actor_user_id: &str,
        session_id: &str,
    ) -> Option<crate::LibraryScanHintSet> {
        let state = self.state.lock().await;
        state.sessions_by_id.get(session_id).and_then(|handle| {
            (handle.actor_user_id == actor_user_id)
                .then(|| handle.scan_hints.clone())
                .flatten()
        })
    }

    pub async fn set_arr_source_result(
        &self,
        session_id: &str,
        result: ExternalImportArrSourceWarmupResult,
    ) -> bool {
        let mut state = self.state.lock().await;
        let Some(handle) = state.sessions_by_id.get_mut(session_id) else {
            return false;
        };
        handle.arr_source_result = Some(result);
        true
    }

    pub async fn arr_source_result(
        &self,
        actor_user_id: &str,
        session_id: &str,
    ) -> Option<ExternalImportArrSourceWarmupResult> {
        let state = self.state.lock().await;
        state.sessions_by_id.get(session_id).and_then(|handle| {
            (handle.actor_user_id == actor_user_id)
                .then(|| handle.arr_source_result.clone())
                .flatten()
        })
    }

    pub async fn set_prowlarr_result(
        &self,
        session_id: &str,
        result: ExternalImportProwlarrWarmupResult,
    ) -> bool {
        let mut state = self.state.lock().await;
        let Some(handle) = state.sessions_by_id.get_mut(session_id) else {
            return false;
        };
        handle.prowlarr_result = Some(result);
        true
    }

    pub async fn prowlarr_result(
        &self,
        actor_user_id: &str,
        session_id: &str,
    ) -> Option<ExternalImportProwlarrWarmupResult> {
        let state = self.state.lock().await;
        state.sessions_by_id.get(session_id).and_then(|handle| {
            (handle.actor_user_id == actor_user_id)
                .then(|| handle.prowlarr_result.clone())
                .flatten()
        })
    }

    pub async fn cancel(&self, actor_user_id: &str, session_id: &str) -> bool {
        let mut state = self.state.lock().await;
        let Some(handle) = state.sessions_by_id.get_mut(session_id) else {
            return false;
        };
        if handle.actor_user_id != actor_user_id || handle.claimed {
            return false;
        }

        let mut snapshot = handle.tx.borrow().clone();
        if !snapshot.status.is_terminal() {
            snapshot.status = ExternalImportMonitorWarmupStatus::Canceled;
            snapshot.error_message = None;
            snapshot.touch();
            handle.tx.send_replace(snapshot);
        }
        handle.cancel_token.cancel();

        state
            .session_ids_by_actor_fingerprint
            .retain(|_, existing_session_id| existing_session_id != session_id);
        true
    }

    pub async fn claim(
        &self,
        actor_user_id: &str,
        session_id: &str,
    ) -> Option<ExternalImportMonitorWarmupProgressSnapshot> {
        let mut state = self.state.lock().await;
        let handle = state.sessions_by_id.get_mut(session_id)?;
        if handle.actor_user_id != actor_user_id {
            return None;
        }
        handle.claimed = true;
        Some(handle.tx.borrow().clone())
    }

    pub async fn connection_fingerprint(
        &self,
        actor_user_id: &str,
        session_id: &str,
    ) -> Option<String> {
        let state = self.state.lock().await;
        state.sessions_by_id.get(session_id).and_then(|handle| {
            (handle.actor_user_id == actor_user_id).then(|| handle.connection_fingerprint.clone())
        })
    }

    pub async fn remove(&self, actor_user_id: &str, session_id: &str) -> bool {
        let mut state = self.state.lock().await;
        let Some(handle) = state.sessions_by_id.get(session_id) else {
            return false;
        };
        if handle.actor_user_id != actor_user_id {
            return false;
        }
        state.sessions_by_id.remove(session_id);
        state
            .session_ids_by_actor_fingerprint
            .retain(|_, existing_session_id| existing_session_id != session_id);
        true
    }

    pub async fn prune_terminal_older_than(&self, max_age: chrono::Duration) -> Vec<String> {
        let mut state = self.state.lock().await;
        let now = Utc::now();
        let mut removed = Vec::new();
        let session_ids = state.sessions_by_id.keys().cloned().collect::<Vec<_>>();

        for session_id in session_ids {
            let Some(handle) = state.sessions_by_id.get(&session_id) else {
                continue;
            };
            if !handle.connection_fingerprint.starts_with("arr-source=")
                && !handle
                    .connection_fingerprint
                    .starts_with("prowlarr-source=")
            {
                continue;
            }
            let snapshot = handle.tx.borrow().clone();
            if !snapshot.status.is_terminal() {
                continue;
            }
            let Ok(updated_at) = chrono::DateTime::parse_from_rfc3339(&snapshot.updated_at) else {
                continue;
            };
            if now.signed_duration_since(updated_at.with_timezone(&Utc)) < max_age {
                continue;
            }

            state.sessions_by_id.remove(&session_id);
            state
                .session_ids_by_actor_fingerprint
                .retain(|_, existing_session_id| existing_session_id != &session_id);
            removed.push(session_id);
        }

        removed
    }
}
