use super::*;
use crate::library_scan_coordinator::{
    LibraryScanCoordinator, load_projected_library_scan_session,
};

const LIBRARY_SCAN_DISCOVERY_WORK_QUEUE_CAPACITY: usize = 16;

pub(crate) fn spawn_library_discovery_queue<T>(
    app: AppUseCase,
    session_id: String,
    mut discovered_batches: tokio::sync::mpsc::Receiver<AppResult<Vec<T>>>,
    track_file_total: bool,
) -> tokio::sync::mpsc::Receiver<AppResult<Vec<T>>>
where
    T: Send + 'static,
{
    let (queued_batches_tx, queued_batches_rx) =
        tokio::sync::mpsc::channel(LIBRARY_SCAN_DISCOVERY_WORK_QUEUE_CAPACITY);

    tokio::spawn(async move {
        let coordinator = LibraryScanCoordinator::new(app.clone(), session_id.clone());
        while let Some(batch_result) = discovered_batches.recv().await {
            let batch = match batch_result {
                Ok(batch) => batch,
                Err(error) => {
                    let _ = queued_batches_tx.send(Err(error)).await;
                    return;
                }
            };

            if batch.is_empty() {
                continue;
            }

            coordinator
                .register_discovery_batch(batch.len(), track_file_total)
                .await;
            if queued_batches_tx.send(Ok(batch)).await.is_err() {
                return;
            }
        }

        coordinator.mark_discovery_complete(track_file_total).await;
    });

    queued_batches_rx
}

pub(crate) fn require_directory_library_path(library_path: &str) -> AppResult<&Path> {
    let root = Path::new(library_path);
    if !root.is_dir() {
        return Err(AppError::Validation(format!(
            "library path is not a directory: {library_path}"
        )));
    }

    Ok(root)
}

pub(crate) struct LibraryScanSessionDropGuard {
    app: AppUseCase,
    session_id: String,
    armed: bool,
}

impl LibraryScanSessionDropGuard {
    pub(crate) fn new(app: AppUseCase, session_id: String) -> Self {
        Self {
            app,
            session_id,
            armed: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for LibraryScanSessionDropGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        let app = self.app.clone();
        let session_id = self.session_id.clone();
        tokio::spawn(async move {
            LibraryScanCoordinator::new(app, session_id).fail().await;
        });
    }
}

pub(crate) async fn wait_for_projected_library_scan_session(
    app: &AppUseCase,
    session_id: &str,
) -> AppResult<LibraryScanSession> {
    let mut receiver = app.services.library_scan_tracker.subscribe();

    loop {
        if let Some(session) = app
            .services
            .library_scan_tracker
            .get_session(session_id)
            .await
        {
            if session.status == LibraryScanStatus::Failed
                || matches!(
                    session.status,
                    LibraryScanStatus::Completed | LibraryScanStatus::Warning
                )
                || session.is_ready_to_complete()
            {
                return Ok(session);
            }
        }

        let projected_session = load_projected_library_scan_session(app, session_id).await?;
        if let Some(session) = projected_session {
            if session.status == LibraryScanStatus::Failed
                || matches!(
                    session.status,
                    LibraryScanStatus::Completed | LibraryScanStatus::Warning
                )
                || session.is_ready_to_complete()
            {
                return Ok(session);
            }
        }

        match receiver.recv().await {
            Ok(session) => {
                if session.session_id == session_id
                    && (session.status == LibraryScanStatus::Failed
                        || matches!(
                            session.status,
                            LibraryScanStatus::Completed | LibraryScanStatus::Warning
                        )
                        || session.is_ready_to_complete())
                {
                    return Ok(session);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                if let Some(session) = load_projected_library_scan_session(app, session_id).await? {
                    if session.status == LibraryScanStatus::Failed
                        || matches!(
                            session.status,
                            LibraryScanStatus::Completed | LibraryScanStatus::Warning
                        )
                        || session.is_ready_to_complete()
                    {
                        return Ok(session);
                    }
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return Err(AppError::Repository(
                    "library scan tracker broadcast closed before completion".into(),
                ));
            }
        }
    }
}
