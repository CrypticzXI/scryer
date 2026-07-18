use super::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{debug, info, warn};

const IMAGE_COLLECT_WINDOW: Duration = Duration::from_millis(50);
const IMAGE_MAX_BATCH: usize = 256;
const IMAGE_WRITE_CHUNK_SIZE: usize = 8;

pub async fn start_background_title_image_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    start_background_image_loop(app, token).await
}

async fn wait_for_image_loop_to_resume(
    app: &AppUseCase,
    token: &tokio_util::sync::CancellationToken,
) -> bool {
    let active_scans = app.runtime.library.library_scan_tracker.list_active().await;
    if active_scans.is_empty() {
        return true;
    }

    debug!(
        active_scans = active_scans.len(),
        "image loop: pausing while library scan is active"
    );

    tokio::select! {
        _ = token.cancelled() => false,
        _ = app.runtime.library.library_scan_tracker.wait_until_idle() => {
            debug!("image loop: resuming after library scan");
            true
        }
    }
}

fn image_task_label(task: &TitleImageSyncTask) -> String {
    let variants = task
        .variants
        .iter()
        .map(|variant| variant.variant_key.as_str())
        .collect::<Vec<_>>()
        .join(",");
    format!("{}:{variants}", task.kind.as_str())
}

async fn process_image_refresh_chunk(
    app: &AppUseCase,
    chunk: &[TitleImageSyncTask],
) -> (usize, Vec<TitleImageSyncTask>, usize) {
    let mut join_set = tokio::task::JoinSet::new();

    for task in chunk.iter().cloned() {
        let app = app.clone();
        join_set.spawn(async move {
            let _permit = app
                .runtime
                .catalog
                .image_processing_limit
                .clone()
                .acquire_owned()
                .await
                .expect("image processing semaphore should not be closed");
            let started_at = std::time::Instant::now();
            let label = image_task_label(&task);
            match task.kind {
                TitleImageKind::Poster => debug!(
                    title_id = %task.title_id,
                    poster_url = %task.source_url,
                    target = %label,
                    "image loop: refreshing"
                ),
                TitleImageKind::Fanart => debug!(
                    title_id = %task.title_id,
                    background_url = %task.source_url,
                    target = %label,
                    "image loop: refreshing"
                ),
            }
            match app
                .services
                .library
                .title_image_processor
                .fetch_and_process_image(task.kind, &task.source_url, task.variants.clone())
                .await
            {
                Ok(result) => {
                    match app
                        .services
                        .library
                        .title_images
                        .upsert_title_image_source_result(&task.title_id, result, None)
                        .await
                    {
                        Ok(_) => {}
                        Err(error) => {
                            match task.kind {
                                TitleImageKind::Poster => warn!(
                                    error = %error,
                                    elapsed_ms = started_at.elapsed().as_millis(),
                                    title_id = %task.title_id,
                                    poster_url = %task.source_url,
                                    target = %label,
                                    "image loop: failed to store processed image"
                                ),
                                TitleImageKind::Fanart => warn!(
                                    error = %error,
                                    elapsed_ms = started_at.elapsed().as_millis(),
                                    title_id = %task.title_id,
                                    background_url = %task.source_url,
                                    target = %label,
                                    "image loop: failed to store processed image"
                                ),
                            }
                            return (task, false);
                        }
                    };

                    debug!(
                        elapsed_ms = started_at.elapsed().as_millis(),
                        title_id = %task.title_id,
                        target = %label,
                        "image loop: cached"
                    );
                    (task, true)
                }
                Err(error) => {
                    match task.kind {
                        TitleImageKind::Poster => warn!(
                            error = %error,
                            title_id = %task.title_id,
                            poster_url = %task.source_url,
                            target = %label,
                            "image loop: fetch/process failed"
                        ),
                        TitleImageKind::Fanart => warn!(
                            error = %error,
                            title_id = %task.title_id,
                            background_url = %task.source_url,
                            target = %label,
                            "image loop: fetch/process failed"
                        ),
                    }
                    (task, false)
                }
            }
        });
    }

    let mut succeeded = 0usize;
    let mut failed = Vec::new();
    let mut unknown_failures = 0usize;
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok((_, true)) => {
                succeeded += 1;
            }
            Ok((task, false)) => {
                failed.push(task);
            }
            Err(err) => {
                warn!(error = %err, "image loop: task panicked");
                unknown_failures += 1;
            }
        }
    }

    (succeeded, failed, unknown_failures)
}

async fn start_background_image_loop(app: AppUseCase, token: tokio_util::sync::CancellationToken) {
    let poster_wake: Arc<Notify> = app.runtime.catalog.poster_wake.clone();
    let fanart_wake: Arc<Notify> = app.runtime.catalog.fanart_wake.clone();

    info!(
        collect_window_ms = IMAGE_COLLECT_WINDOW.as_millis(),
        max_batch = IMAGE_MAX_BATCH,
        write_chunk_size = IMAGE_WRITE_CHUNK_SIZE,
        shared_concurrent_workers = app
            .runtime
            .catalog
            .image_processing_limit
            .available_permits(),
        "background title image loop started"
    );

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                info!("background title image loop shutting down");
                return;
            }
            _ = poster_wake.notified() => {}
            _ = fanart_wake.notified() => {}
        }

        let mut skipped_work = Vec::new();
        'drain: loop {
            if !wait_for_image_loop_to_resume(&app, &token).await {
                return;
            }

            tokio::select! {
                _ = token.cancelled() => return,
                _ = tokio::time::sleep(IMAGE_COLLECT_WINDOW) => {}
            }

            if !wait_for_image_loop_to_resume(&app, &token).await {
                return;
            }

            let (batch_len, succeeded, failed, unknown_failures) = {
                let batch_result = {
                    let _maintenance_guard = tokio::select! {
                        _ = token.cancelled() => return,
                        guard = app.runtime.catalog.title_image_maintenance_lock.read() => guard,
                    };

                    app.services
                        .library
                        .title_images
                        .list_title_image_refresh_work(IMAGE_MAX_BATCH, &skipped_work)
                        .await
                };
                let batch = match batch_result {
                    Ok(batch) => batch,
                    Err(error) => {
                        warn!(error = %error, "image loop: failed to list pending image sync work");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue 'drain;
                    }
                };

                if batch.is_empty() {
                    debug!("image loop: no pending work");
                    break 'drain;
                }

                let batch_len = batch.len();
                debug!(
                    count = batch_len,
                    target = %image_task_label(&batch[0]),
                    "image loop: processing batch"
                );

                let mut succeeded = 0usize;
                let mut failed = Vec::new();
                let mut unknown_failures = 0usize;
                for chunk in batch.chunks(IMAGE_WRITE_CHUNK_SIZE) {
                    if !wait_for_image_loop_to_resume(&app, &token).await {
                        return;
                    }

                    let _maintenance_guard = tokio::select! {
                        _ = token.cancelled() => return,
                        guard = app.runtime.catalog.title_image_maintenance_lock.read() => guard,
                    };
                    let (chunk_succeeded, mut chunk_failed, chunk_unknown_failures) =
                        process_image_refresh_chunk(&app, chunk).await;
                    succeeded += chunk_succeeded;
                    failed.append(&mut chunk_failed);
                    unknown_failures += chunk_unknown_failures;
                }

                let failed_count = failed.len() + unknown_failures;
                debug!(
                    processed = batch_len,
                    succeeded,
                    failed = failed_count,
                    skipped_until_next_wake = failed.len(),
                    target = %image_task_label(&batch[0]),
                    "image loop: batch complete"
                );

                (batch_len, succeeded, failed, unknown_failures)
            };

            if !failed.is_empty() || unknown_failures > 0 {
                info!(
                    processed = batch_len,
                    succeeded,
                    failed = failed.len() + unknown_failures,
                    skipped_until_next_wake = failed.len(),
                    unknown_failures,
                    "image loop: some images failed, skipping failed work until next wake"
                );
                skipped_work.extend(failed);
            }
        }

        debug!(
            skipped_until_next_wake = skipped_work.len(),
            "image loop: queue drained, parking"
        );
    }
}
