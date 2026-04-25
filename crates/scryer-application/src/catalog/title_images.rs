use super::*;
use crate::domain_events::{new_title_domain_event, title_context_snapshot};
use scryer_domain::{DomainEventPayload, TitleUpdatedEventData};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{debug, info, warn};

const IMAGE_COLLECT_WINDOW: Duration = Duration::from_millis(50);
const IMAGE_MAX_BATCH: usize = 256;
const IMAGE_WRITE_CHUNK_SIZE: usize = 8;
const IMAGE_RETRY_BASE: Duration = Duration::from_secs(10);
const IMAGE_RETRY_MAX: Duration = Duration::from_secs(300);
const IMAGE_CONCURRENT_WORKERS: usize = 2;

pub async fn start_background_poster_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    start_background_image_loop(app, token, TitleImageKind::Poster).await
}

pub async fn start_background_banner_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    start_background_image_loop(app, token, TitleImageKind::Banner).await
}

pub async fn start_background_fanart_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    start_background_image_loop(app, token, TitleImageKind::Fanart).await
}

async fn wait_for_image_loop_to_resume(
    app: &AppUseCase,
    token: &tokio_util::sync::CancellationToken,
    kind: TitleImageKind,
) -> bool {
    let active_scans = app.runtime.library.library_scan_tracker.list_active().await;
    if active_scans.is_empty() {
        return true;
    }

    debug!(
        kind = kind.as_str(),
        active_scans = active_scans.len(),
        "image loop: pausing while library scan is active"
    );

    tokio::select! {
        _ = token.cancelled() => false,
        _ = app.runtime.library.library_scan_tracker.wait_until_idle() => {
            debug!(kind = kind.as_str(), "image loop: resuming after library scan");
            true
        }
    }
}

async fn process_image_refresh_chunk(
    app: &AppUseCase,
    kind: TitleImageKind,
    label: &str,
    chunk: &[TitleImageSyncTask],
) -> (usize, usize) {
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(IMAGE_CONCURRENT_WORKERS));
    let mut join_set = tokio::task::JoinSet::new();
    let label = label.to_string();

    for task in chunk.iter().cloned() {
        let sem = semaphore.clone();
        let app = app.clone();
        let label = label.clone();
        join_set.spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore should not be closed");
            let started_at = std::time::Instant::now();
            debug!(
                title_id = %task.title_id,
                source_url = %task.source_url,
                kind = %label,
                "image loop: refreshing"
            );
            match app
                .services
                .library
                .title_image_processor
                .fetch_and_process_image(kind, &task.source_url)
                .await
            {
                Ok(replacement) => {
                    let stored_event = if kind == TitleImageKind::Poster {
                        let title = match app
                            .services
                            .catalog
                            .titles
                            .get_by_id(&task.title_id)
                            .await
                        {
                            Ok(Some(title)) => title,
                            Ok(None) => {
                                warn!(
                                    elapsed_ms = started_at.elapsed().as_millis(),
                                    title_id = %task.title_id,
                                    kind = %label,
                                    "image loop: cached image for missing title"
                                );
                                return false;
                            }
                            Err(error) => {
                                warn!(
                                    error = %error,
                                    elapsed_ms = started_at.elapsed().as_millis(),
                                    title_id = %task.title_id,
                                    kind = %label,
                                    "image loop: failed to load title for cached image refresh event"
                                );
                                return false;
                            }
                        };
                        let event = new_title_domain_event(
                            None,
                            &title,
                            DomainEventPayload::TitleUpdated(TitleUpdatedEventData {
                                title: title_context_snapshot(&title),
                            }),
                        );
                        match app
                            .services
                            .library
                            .title_images
                            .replace_title_image_and_append_event(
                                &task.title_id,
                                replacement,
                                event,
                            )
                            .await
                        {
                            Ok(event) => Some(event),
                            Err(error) => {
                                warn!(
                                    error = %error,
                                    elapsed_ms = started_at.elapsed().as_millis(),
                                    title_id = %task.title_id,
                                    source_url = %task.source_url,
                                    kind = %label,
                                    "image loop: failed to store processed image and append refresh event"
                                );
                                return false;
                            }
                        }
                    } else {
                        if let Err(error) = app
                            .services
                            .library
                            .title_images
                            .replace_title_image(&task.title_id, replacement)
                            .await
                        {
                            warn!(
                                error = %error,
                                elapsed_ms = started_at.elapsed().as_millis(),
                                title_id = %task.title_id,
                                source_url = %task.source_url,
                                kind = %label,
                                "image loop: failed to store processed image"
                            );
                            return false;
                        }
                        None
                    };

                    debug!(
                        elapsed_ms = started_at.elapsed().as_millis(),
                        title_id = %task.title_id,
                        kind = %label,
                        "image loop: cached"
                    );
                    if let Some(event) = stored_event {
                        app.publish_stored_domain_event(&event).await;
                    }
                    true
                }
                Err(error) => {
                    warn!(
                        error = %error,
                        title_id = %task.title_id,
                        source_url = %task.source_url,
                        kind = %label,
                        "image loop: fetch/process failed"
                    );
                    false
                }
            }
        });
    }

    let mut succeeded = 0usize;
    let mut failed = 0usize;
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(true) => {
                succeeded += 1;
            }
            Ok(false) => {
                failed += 1;
            }
            Err(err) => {
                warn!(error = %err, kind = label, "image loop: task panicked");
                failed += 1;
            }
        }
    }

    (succeeded, failed)
}

async fn start_background_image_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
    kind: TitleImageKind,
) {
    let label: &'static str = kind.as_str();
    let wake: Arc<Notify> = match kind {
        TitleImageKind::Poster => app.runtime.catalog.poster_wake.clone(),
        TitleImageKind::Banner => app.runtime.catalog.banner_wake.clone(),
        TitleImageKind::Fanart => app.runtime.catalog.fanart_wake.clone(),
    };

    info!(
        kind = label,
        collect_window_ms = IMAGE_COLLECT_WINDOW.as_millis(),
        max_batch = IMAGE_MAX_BATCH,
        write_chunk_size = IMAGE_WRITE_CHUNK_SIZE,
        concurrent_workers = IMAGE_CONCURRENT_WORKERS,
        retry_base_secs = IMAGE_RETRY_BASE.as_secs(),
        retry_max_secs = IMAGE_RETRY_MAX.as_secs(),
        "background image loop started"
    );

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                info!(kind = label, "background image loop shutting down");
                return;
            }
            _ = wake.notified() => {}
        }

        let mut retry_delay = IMAGE_RETRY_BASE;
        'drain: loop {
            if !wait_for_image_loop_to_resume(&app, &token, kind).await {
                return;
            }

            tokio::select! {
                _ = token.cancelled() => return,
                _ = tokio::time::sleep(IMAGE_COLLECT_WINDOW) => {}
            }

            if !wait_for_image_loop_to_resume(&app, &token, kind).await {
                return;
            }

            let batch = match app
                .services
                .library
                .title_images
                .list_titles_requiring_image_refresh(kind, IMAGE_MAX_BATCH)
                .await
            {
                Ok(batch) => batch,
                Err(error) => {
                    warn!(error = %error, kind = label, "image loop: failed to list pending image sync work");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue 'drain;
                }
            };

            if batch.is_empty() {
                debug!(kind = label, "image loop: no pending work");
                break 'drain;
            }

            let batch_len = batch.len();
            debug!(
                count = batch_len,
                kind = label,
                "image loop: processing batch"
            );

            let mut succeeded = 0usize;
            let mut failed = 0usize;
            for chunk in batch.chunks(IMAGE_WRITE_CHUNK_SIZE) {
                if !wait_for_image_loop_to_resume(&app, &token, kind).await {
                    return;
                }

                let (chunk_succeeded, chunk_failed) =
                    process_image_refresh_chunk(&app, kind, label, chunk).await;
                succeeded += chunk_succeeded;
                failed += chunk_failed;
            }

            let had_failures = failed > 0;

            debug!(
                processed = batch_len,
                succeeded,
                failed,
                kind = label,
                "image loop: batch complete"
            );

            if had_failures {
                info!(
                    retry_secs = retry_delay.as_secs(),
                    processed = batch_len,
                    succeeded,
                    failed,
                    kind = label,
                    "image loop: some images failed, scheduling retry"
                );
                let new_work = tokio::select! {
                    _ = token.cancelled() => return,
                    _ = tokio::time::sleep(retry_delay) => false,
                    _ = wake.notified() => true,
                };

                if new_work {
                    retry_delay = IMAGE_RETRY_BASE;
                } else {
                    retry_delay = (retry_delay * 2).min(IMAGE_RETRY_MAX);
                }

                continue 'drain;
            }
        }

        debug!(kind = label, "image loop: queue drained, parking");
    }
}
