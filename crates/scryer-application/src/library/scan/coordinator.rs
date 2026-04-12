use crate::domain_events::new_library_scan_domain_event;
use crate::{
    AppResult, AppUseCase, Id, LibraryScanMode, LibraryScanSession, LibraryScanSummary,
    library_scan_progress::reduce_library_scan_projection_event,
};
use scryer_domain::{
    DomainEventFilter, DomainEventPayload, DomainEventType, LibraryScanCanceledEventData,
    LibraryScanCompletedEventData, LibraryScanDeltaRecordedEventData, LibraryScanFailedEventData,
    LibraryScanProgressedEventData, LibraryScanStartedEventData, LibraryScanSummaryEventData,
    MediaFacet, NewDomainEvent,
};
use tracing::{debug, trace, warn};

const LIBRARY_SCAN_TRACKER_EVENT_TYPES: &[DomainEventType] = &[
    DomainEventType::LibraryScanStarted,
    DomainEventType::LibraryScanTitleDiscovered,
    DomainEventType::LibraryScanDeltaRecorded,
    DomainEventType::LibraryScanProgressed,
    DomainEventType::LibraryScanCompleted,
    DomainEventType::LibraryScanCanceled,
    DomainEventType::LibraryScanFailed,
];

#[derive(Clone)]
pub(crate) struct LibraryScanCoordinator {
    app: AppUseCase,
    session_id: String,
    facet: Option<MediaFacet>,
}

impl LibraryScanCoordinator {
    pub(crate) async fn start(
        app: AppUseCase,
        facet: MediaFacet,
        mode: LibraryScanMode,
        session_id_override: Option<String>,
    ) -> AppResult<(Self, LibraryScanSession)> {
        let session_id = session_id_override.unwrap_or_else(|| Id::new().0);
        let session = app
            .runtime
            .library_scan_tracker
            .start_session_with_id(session_id, facet.clone(), mode)
            .await?;
        let coordinator = Self::with_facet(app, session.session_id.clone(), facet);
        coordinator.publish_started(&session).await;
        Ok((coordinator, session))
    }

    pub(crate) fn new(app: AppUseCase, session_id: impl Into<String>) -> Self {
        Self {
            app,
            session_id: session_id.into(),
            facet: None,
        }
    }

    pub(crate) fn with_facet(
        app: AppUseCase,
        session_id: impl Into<String>,
        facet: MediaFacet,
    ) -> Self {
        Self {
            app,
            session_id: session_id.into(),
            facet: Some(facet),
        }
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) async fn publish_started(&self, session: &LibraryScanSession) {
        let _ = self
            .app
            .append_domain_event(new_library_scan_domain_event(
                None,
                session.session_id.clone(),
                session.facet.clone(),
                DomainEventPayload::LibraryScanStarted(LibraryScanStartedEventData {
                    session_id: session.session_id.clone(),
                    mode: session.mode.as_str().to_string(),
                }),
            ))
            .await;
    }

    pub(crate) async fn publish_progress(&self) {
        let Some(session) = self
            .app
            .runtime
            .library_scan_tracker
            .get_session(self.session_id())
            .await
        else {
            trace!(
                session_id = %self.session_id,
                "library scan coordinator publish_progress skipped for inactive session"
            );
            return;
        };

        if session.is_ready_to_complete() {
            trace!(
                session_id = %self.session_id,
                "library scan coordinator publish_progress deferred to completion path"
            );
            return;
        }

        publish_coalesced_library_scan_state(&self.app, &session).await;
    }

    pub(crate) async fn register_discovery_batch(
        &self,
        discovered_count: usize,
        track_file_total: bool,
    ) {
        if discovered_count == 0 {
            return;
        }

        let mut delta = empty_scan_delta(self.session_id.clone());
        delta.found_titles_delta = discovered_count as i64;
        if track_file_total {
            delta.file_total_delta = discovered_count as i64;
        }
        self.record_delta(delta).await;
    }

    pub(crate) async fn mark_metadata_total_known(&self) {
        let mut delta = empty_scan_delta(self.session_id.clone());
        delta.metadata_total_known = Some(true);
        self.record_delta(delta).await;
    }

    pub(crate) async fn add_metadata_total(&self, additional: usize) {
        if additional == 0 {
            return;
        }

        let mut delta = empty_scan_delta(self.session_id.clone());
        delta.metadata_total_delta = additional as i64;
        self.record_delta(delta).await;
    }

    pub(crate) async fn add_file_total(&self, additional: usize) {
        if additional == 0 {
            return;
        }

        let mut delta = empty_scan_delta(self.session_id.clone());
        delta.file_total_delta = additional as i64;
        self.record_delta(delta).await;
    }

    pub(crate) async fn mark_file_total_known(&self) {
        let mut delta = empty_scan_delta(self.session_id.clone());
        delta.file_total_known = Some(true);
        self.record_delta(delta).await;
    }

    pub(crate) async fn mark_metadata_completed(&self, additional: usize) {
        if additional == 0 {
            return;
        }

        let mut delta = empty_scan_delta(self.session_id.clone());
        delta.metadata_completed_delta = additional as i64;
        self.record_delta(delta).await;
    }

    pub(crate) async fn mark_metadata_failed(&self, additional: usize) {
        if additional == 0 {
            return;
        }

        let mut delta = empty_scan_delta(self.session_id.clone());
        delta.metadata_failed_delta = additional as i64;
        self.record_delta(delta).await;
    }

    pub(crate) async fn mark_file_completed(&self, additional: usize) {
        if additional == 0 {
            return;
        }

        let mut delta = empty_scan_delta(self.session_id.clone());
        delta.file_completed_delta = additional as i64;
        self.record_delta(delta).await;
    }

    pub(crate) async fn mark_file_failed(&self, additional: usize) {
        if additional == 0 {
            return;
        }

        let mut delta = empty_scan_delta(self.session_id.clone());
        delta.file_failed_delta = additional as i64;
        self.record_delta(delta).await;
    }

    pub(crate) async fn mark_discovery_complete(&self, track_file_total: bool) {
        let mut delta = empty_scan_delta(self.session_id.clone());
        delta.title_match_total_known = Some(true);
        if track_file_total {
            delta.file_total_known = Some(true);
        }
        self.record_delta(delta).await;
    }

    pub(crate) async fn mark_title_match_completed(&self, additional: usize) {
        if additional == 0 {
            return;
        }

        let mut delta = empty_scan_delta(self.session_id.clone());
        delta.title_match_completed_delta = additional as i64;
        self.record_delta(delta).await;
    }

    pub(crate) async fn set_summary(&self, summary: LibraryScanSummary) {
        let mut delta = empty_scan_delta(self.session_id.clone());
        delta.summary = Some(library_scan_summary_event_data(&summary));
        self.record_delta(delta).await;
    }

    pub(crate) async fn maybe_complete(&self) {
        let Some(session) = self
            .app
            .runtime
            .library_scan_tracker
            .complete_if_finished(self.session_id())
            .await
        else {
            trace!(
                session_id = %self.session_id,
                "library scan coordinator maybe_complete found no terminal session"
            );
            return;
        };

        self.app
            .clear_library_scan_cancellation_token(self.session_id())
            .await;
        publish_coalesced_library_scan_state(&self.app, &session).await;
    }

    pub(crate) async fn fail(&self) {
        let failed_session = self
            .app
            .runtime
            .library_scan_tracker
            .fail_session(self.session_id())
            .await;
        self.app
            .clear_library_scan_cancellation_token(self.session_id())
            .await;
        let facet = match failed_session.as_ref().map(|session| session.facet.clone()) {
            Some(facet) => facet,
            None => {
                let Some(facet) = self.resolve_facet().await else {
                    warn!(session_id = %self.session_id, "failed to resolve facet for library scan failure event");
                    return;
                };
                facet
            }
        };

        let _ = self
            .app
            .append_domain_event(self.scan_event(
                facet,
                DomainEventPayload::LibraryScanFailed(LibraryScanFailedEventData {
                    session_id: self.session_id.clone(),
                    error_message: "library scan failed".to_string(),
                }),
            ))
            .await;
    }

    pub(crate) async fn cancel(&self) {
        let canceled_session = self
            .app
            .runtime
            .library_scan_tracker
            .cancel_session(self.session_id())
            .await;
        self.app
            .clear_library_scan_cancellation_token(self.session_id())
            .await;
        let Some(session) = canceled_session else {
            trace!(
                session_id = %self.session_id,
                "library scan coordinator cancel skipped for inactive session"
            );
            return;
        };

        let _ = self
            .app
            .append_domain_event(self.scan_event(
                session.facet.clone(),
                DomainEventPayload::LibraryScanCanceled(library_scan_canceled_event_data(&session)),
            ))
            .await;
    }

    fn scan_event(&self, facet: MediaFacet, payload: DomainEventPayload) -> NewDomainEvent {
        new_library_scan_domain_event(None, self.session_id.clone(), facet, payload)
    }

    async fn record_delta(&self, delta: LibraryScanDeltaRecordedEventData) {
        if !delta_has_effect(&delta) {
            return;
        }

        let Some(snapshot) = self
            .app
            .runtime
            .library_scan_tracker
            .apply_delta(self.session_id(), &delta)
            .await
        else {
            warn!(session_id = %self.session_id, "ignored library scan delta for inactive session");
            return;
        };

        debug!(
            session_id = %self.session_id,
            facet = %snapshot.facet.as_str(),
            found_titles_total = ?delta.found_titles_total,
            found_titles_delta = delta.found_titles_delta,
            title_match_completed_delta = delta.title_match_completed_delta,
            title_match_failed_delta = delta.title_match_failed_delta,
            title_match_total_known = ?delta.title_match_total_known,
            metadata_total_delta = delta.metadata_total_delta,
            metadata_completed_delta = delta.metadata_completed_delta,
            metadata_failed_delta = delta.metadata_failed_delta,
            metadata_total_known = ?delta.metadata_total_known,
            file_total_delta = delta.file_total_delta,
            file_completed_delta = delta.file_completed_delta,
            file_failed_delta = delta.file_failed_delta,
            file_total_known = ?delta.file_total_known,
            summary_present = delta.summary.is_some(),
            summary_is_delta = delta.summary_is_delta,
            "library scan coordinator recording delta"
        );

        let _ = self
            .app
            .append_domain_event(self.scan_event(
                snapshot.facet,
                DomainEventPayload::LibraryScanDeltaRecorded(delta),
            ))
            .await;
    }

    async fn resolve_facet(&self) -> Option<MediaFacet> {
        if let Some(facet) = self.facet.clone() {
            return Some(facet);
        }

        if let Some(session) = self
            .app
            .runtime
            .library_scan_tracker
            .get_session(self.session_id())
            .await
        {
            return Some(session.facet);
        }

        load_library_scan_session_facet(&self.app, self.session_id()).await
    }
}

pub(crate) async fn load_projected_library_scan_session(
    app: &AppUseCase,
    session_id: &str,
) -> AppResult<Option<LibraryScanSession>> {
    let mut after_sequence = 0i64;
    let mut sessions = std::collections::HashMap::new();
    let mut last_snapshot = None;

    loop {
        let batch = app
            .services
            .events
            .domain_events
            .list(&DomainEventFilter {
                event_types: Some(LIBRARY_SCAN_TRACKER_EVENT_TYPES.to_vec()),
                after_sequence: Some(after_sequence),
                limit: 500,
                ..DomainEventFilter::default()
            })
            .await?;
        if batch.is_empty() {
            break;
        }

        after_sequence = batch
            .last()
            .map(|event| event.sequence)
            .unwrap_or(after_sequence);
        let count = batch.len();
        for event in batch {
            if library_scan_event_session_id(&event.payload) == Some(session_id) {
                last_snapshot = reduce_library_scan_projection_event(&mut sessions, &event);
            }
        }
        if count < 500 {
            break;
        }
    }

    Ok(last_snapshot)
}

async fn publish_coalesced_library_scan_state(app: &AppUseCase, session: &LibraryScanSession) {
    let ready_to_complete = session.is_ready_to_complete();
    if ready_to_complete {
        debug!(
            session_id = %session.session_id,
            facet = %session.facet.as_str(),
            status = %session.status.as_str(),
            found_titles = session.found_titles,
            title_match_total = session.title_match_progress.total,
            title_match_completed = session.title_match_progress.completed,
            title_match_failed = session.title_match_progress.failed,
            metadata_total = session.metadata_progress.total,
            metadata_completed = session.metadata_progress.completed,
            metadata_failed = session.metadata_progress.failed,
            file_total = session.file_progress.total,
            file_completed = session.file_progress.completed,
            file_failed = session.file_progress.failed,
            summary_present = session.summary.is_some(),
            emitted_event_type = "library_scan_completed",
            "library scan coordinator publishing coalesced state"
        );
    } else {
        trace!(
            session_id = %session.session_id,
            facet = %session.facet.as_str(),
            status = %session.status.as_str(),
            found_titles = session.found_titles,
            title_match_total_known = session.title_match_total_known,
            title_match_total = session.title_match_progress.total,
            title_match_completed = session.title_match_progress.completed,
            title_match_failed = session.title_match_progress.failed,
            metadata_total_known = session.metadata_total_known,
            metadata_total = session.metadata_progress.total,
            metadata_completed = session.metadata_progress.completed,
            metadata_failed = session.metadata_progress.failed,
            file_total_known = session.file_total_known,
            file_total = session.file_progress.total,
            file_completed = session.file_progress.completed,
            file_failed = session.file_progress.failed,
            summary_present = session.summary.is_some(),
            ready_to_complete,
            emitted_event_type = "library_scan_progressed",
            "library scan coordinator publishing coalesced state"
        );
    }

    let payload = if ready_to_complete {
        DomainEventPayload::LibraryScanCompleted(library_scan_completed_event_data(session))
    } else {
        DomainEventPayload::LibraryScanProgressed(library_scan_progressed_event_data(session))
    };

    let _ = app
        .append_domain_event(new_library_scan_domain_event(
            None,
            session.session_id.clone(),
            session.facet.clone(),
            payload,
        ))
        .await;
}

async fn load_library_scan_session_facet(app: &AppUseCase, session_id: &str) -> Option<MediaFacet> {
    let mut after_sequence = 0i64;

    loop {
        let batch = app
            .services
            .events
            .domain_events
            .list(&DomainEventFilter {
                event_types: Some(LIBRARY_SCAN_TRACKER_EVENT_TYPES.to_vec()),
                after_sequence: Some(after_sequence),
                limit: 500,
                ..DomainEventFilter::default()
            })
            .await
            .ok()?;
        if batch.is_empty() {
            return None;
        }

        after_sequence = batch
            .last()
            .map(|event| event.sequence)
            .unwrap_or(after_sequence);

        for event in batch {
            if library_scan_event_session_id(&event.payload) == Some(session_id)
                && let Some(facet) = event.facet
            {
                return Some(facet);
            }
        }
    }
}

fn empty_scan_delta(session_id: String) -> LibraryScanDeltaRecordedEventData {
    LibraryScanDeltaRecordedEventData {
        session_id,
        found_titles_total: None,
        found_titles_delta: 0,
        title_match_completed_delta: 0,
        title_match_failed_delta: 0,
        title_match_total_known: None,
        metadata_total_delta: 0,
        metadata_completed_delta: 0,
        metadata_failed_delta: 0,
        metadata_total_known: None,
        file_total_delta: 0,
        file_completed_delta: 0,
        file_failed_delta: 0,
        file_total_known: None,
        summary: None,
        summary_is_delta: false,
    }
}

fn delta_has_effect(delta: &LibraryScanDeltaRecordedEventData) -> bool {
    delta.found_titles_total.is_some()
        || delta.found_titles_delta != 0
        || delta.title_match_completed_delta != 0
        || delta.title_match_failed_delta != 0
        || delta.title_match_total_known.is_some()
        || delta.metadata_total_delta != 0
        || delta.metadata_completed_delta != 0
        || delta.metadata_failed_delta != 0
        || delta.metadata_total_known.is_some()
        || delta.file_total_delta != 0
        || delta.file_completed_delta != 0
        || delta.file_failed_delta != 0
        || delta.file_total_known.is_some()
        || delta.summary.is_some()
}

fn library_scan_progressed_event_data(
    session: &LibraryScanSession,
) -> LibraryScanProgressedEventData {
    LibraryScanProgressedEventData {
        session_id: session.session_id.clone(),
        status: session.status.as_str().to_string(),
        found_titles: session.found_titles as i64,
        title_match_completed: session.title_match_progress.completed as i64,
        title_match_total_known: session.title_match_total_known,
        titles_completed: session.metadata_progress.completed as i64,
        titles_total: session
            .metadata_total_known
            .then_some(session.metadata_progress.total as i64),
        files_completed: session.file_progress.completed as i64,
        files_total: session
            .file_total_known
            .then_some(session.file_progress.total as i64),
    }
}

fn library_scan_completed_event_data(
    session: &LibraryScanSession,
) -> LibraryScanCompletedEventData {
    LibraryScanCompletedEventData {
        session_id: session.session_id.clone(),
        status: session.completion_status().as_str().to_string(),
        found_titles: session.found_titles as i64,
        title_match_completed: session.title_match_progress.completed as i64,
        title_match_total_known: true,
        titles_completed: session.metadata_progress.completed as i64,
        titles_total: Some(session.metadata_progress.total as i64),
        files_completed: session.file_progress.completed as i64,
        files_total: Some(session.file_progress.total as i64),
        summary: session
            .summary
            .as_ref()
            .map(library_scan_summary_event_data),
    }
}

fn library_scan_canceled_event_data(session: &LibraryScanSession) -> LibraryScanCanceledEventData {
    LibraryScanCanceledEventData {
        session_id: session.session_id.clone(),
        status: session.status.as_str().to_string(),
        found_titles: session.found_titles as i64,
        title_match_completed: session.title_match_progress.completed as i64,
        title_match_total_known: session.title_match_total_known,
        titles_completed: session.metadata_progress.completed as i64,
        titles_total: Some(session.metadata_progress.total as i64),
        files_completed: session.file_progress.completed as i64,
        files_total: Some(session.file_progress.total as i64),
        summary: session
            .summary
            .as_ref()
            .map(library_scan_summary_event_data),
    }
}

fn library_scan_event_session_id(payload: &DomainEventPayload) -> Option<&str> {
    match payload {
        DomainEventPayload::LibraryScanStarted(data) => Some(&data.session_id),
        DomainEventPayload::LibraryScanTitleDiscovered(data) => Some(&data.session_id),
        DomainEventPayload::LibraryScanDeltaRecorded(data) => Some(&data.session_id),
        DomainEventPayload::LibraryScanProgressed(data) => Some(&data.session_id),
        DomainEventPayload::LibraryScanCompleted(data) => Some(&data.session_id),
        DomainEventPayload::LibraryScanCanceled(data) => Some(&data.session_id),
        DomainEventPayload::LibraryScanFailed(data) => Some(&data.session_id),
        _ => None,
    }
}

fn library_scan_summary_event_data(summary: &LibraryScanSummary) -> LibraryScanSummaryEventData {
    LibraryScanSummaryEventData {
        scanned: summary.scanned as i64,
        matched: summary.matched as i64,
        imported: summary.imported as i64,
        skipped: summary.skipped as i64,
        unmatched: summary.unmatched as i64,
    }
}
