use super::*;
use crate::catalog_workflow::{
    HYDRATION_BULK_BATCH_SIZE, HydrationSource, HydrationTarget, extract_tvdb_id,
};
use crate::polling_worker::PollingWorker;
use std::time::Duration;
use tracing::{debug, info, warn};

const TITLE_HYDRATION_MAX_BATCH: usize = HYDRATION_BULK_BATCH_SIZE;
const TITLE_HYDRATION_IDLE_POLL_INTERVAL: Duration = Duration::from_secs(30);
const TITLE_HYDRATION_RETRY_BASE: Duration = Duration::from_secs(10);
const TITLE_HYDRATION_RETRY_MAX: Duration = Duration::from_secs(300);
const TITLE_HYDRATION_MAX_ATTEMPTS: i64 = 12;
const ANIBRIDGE_SCOPED_ID_BACKFILL_BATCH: usize = 500;

fn active_scan_facet_labels(facets: &[MediaFacet]) -> Vec<&'static str> {
    facets.iter().map(MediaFacet::as_str).collect()
}

pub async fn start_background_title_hydration_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    let worker = PollingWorker::new("title_hydration", token);
    info!(
        max_batch = TITLE_HYDRATION_MAX_BATCH,
        idle_poll_secs = TITLE_HYDRATION_IDLE_POLL_INTERVAL.as_secs(),
        retry_base_secs = TITLE_HYDRATION_RETRY_BASE.as_secs(),
        retry_max_secs = TITLE_HYDRATION_RETRY_MAX.as_secs(),
        max_attempts = TITLE_HYDRATION_MAX_ATTEMPTS,
        "background title hydration loop started"
    );
    queue_missing_anibridge_scoped_id_hydration(&app).await;

    loop {
        let blocked_facets = app
            .runtime
            .library
            .library_scan_tracker
            .active_facets()
            .await;
        let due_titles = match app
            .services
            .catalog
            .titles
            .list_titles_due_for_hydration(TITLE_HYDRATION_MAX_BATCH, &blocked_facets)
            .await
        {
            Ok(due_titles) => due_titles,
            Err(error) => {
                worker.warn_error("list_due_titles", &error);
                if !worker.wait_for_sleep(Duration::from_secs(2)).await {
                    return;
                }
                continue;
            }
        };

        metrics::gauge!("scryer_title_metadata_hydration_pending").set(due_titles.len() as f64);

        if due_titles.is_empty() {
            if blocked_facets.is_empty() {
                if !worker
                    .wait_for_wake_or_timeout(
                        &app.runtime.catalog.title_hydration_wake,
                        TITLE_HYDRATION_IDLE_POLL_INTERVAL,
                    )
                    .await
                {
                    return;
                }
            } else {
                metrics::counter!("scryer_title_metadata_hydration_scan_owned_yields_total")
                    .increment(1);
                debug!(
                    blocked_facets = ?active_scan_facet_labels(&blocked_facets),
                    "title hydration loop: yielding while library scan owns active facet"
                );
                if !worker
                    .wait_for_future_or_wake_or_timeout(
                        &app.runtime.catalog.title_hydration_wake,
                        app.runtime
                            .library
                            .library_scan_tracker
                            .wait_for_active_facets_change(&blocked_facets),
                        TITLE_HYDRATION_IDLE_POLL_INTERVAL,
                    )
                    .await
                {
                    return;
                }
            }

            continue;
        }

        let blocked_facets_before_dispatch = app
            .runtime
            .library
            .library_scan_tracker
            .active_facets()
            .await;
        if blocked_facets_before_dispatch != blocked_facets {
            metrics::counter!("scryer_title_metadata_hydration_scan_owned_rechecks_total")
                .increment(1);
            continue;
        }

        let mut original_attempts = std::collections::HashMap::with_capacity(due_titles.len());
        let mut targets = Vec::with_capacity(due_titles.len());
        for due_title in due_titles {
            original_attempts.insert(
                due_title.title.id.clone(),
                (due_title.attempt_count, due_title.title.facet.clone()),
            );
            if extract_tvdb_id(&due_title.title).is_none() {
                warn!(
                    hydration_source = HydrationSource::BackgroundDue.as_str(),
                    facet = due_title.title.facet.as_str(),
                    title_id = %due_title.title.id,
                    title_name = %due_title.title.name,
                    "title hydration loop: clearing retry state because title has no tvdb external id"
                );
                let _ = app
                    .services
                    .catalog
                    .titles
                    .clear_title_metadata_hydration_retry_state(&due_title.title.id)
                    .await;
                continue;
            }
            targets.push(HydrationTarget {
                title: due_title.title,
                requested_tvdb_id: None,
                sync_wanted_after_completion: true,
                source: HydrationSource::BackgroundDue,
            });
        }

        if targets.is_empty() {
            continue;
        }

        info!(
            hydration_source = HydrationSource::BackgroundDue.as_str(),
            count = targets.len(),
            "title hydration loop: processing batch"
        );

        for _ in 0..targets.len() {
            metrics::counter!("scryer_title_metadata_hydration_attempts_total").increment(1);
        }

        let title_ids: Vec<String> = targets
            .iter()
            .map(|target| target.title.id.clone())
            .collect::<Vec<_>>();

        match app.hydrate_titles_bulk(targets).await {
            Ok(outcome) => {
                for title_id in outcome.hydrated_titles.keys() {
                    metrics::counter!("scryer_title_metadata_hydration_success_total").increment(1);
                    original_attempts.remove(title_id);
                }

                for (title_id, _) in outcome.failed_titles {
                    metrics::counter!("scryer_title_metadata_hydration_failure_total").increment(1);
                    if let Some((previous_attempt_count, facet)) =
                        original_attempts.remove(&title_id)
                    {
                        schedule_title_hydration_retry(
                            &app,
                            &title_id,
                            &facet,
                            previous_attempt_count,
                        )
                        .await;
                    }
                }

                for title_id in original_attempts.keys() {
                    let _ = app
                        .services
                        .catalog
                        .titles
                        .mark_title_metadata_hydration_due_now(title_id)
                        .await;
                }
            }
            Err(error) => {
                warn!(
                    hydration_source = HydrationSource::BackgroundDue.as_str(),
                    error = %error,
                    title_ids = ?title_ids,
                    "title hydration loop: batch failed"
                );
                for title_id in title_ids {
                    metrics::counter!("scryer_title_metadata_hydration_failure_total").increment(1);
                    if let Some((previous_attempt_count, facet)) =
                        original_attempts.get(&title_id).cloned()
                    {
                        schedule_title_hydration_retry(
                            &app,
                            &title_id,
                            &facet,
                            previous_attempt_count,
                        )
                        .await;
                    }
                }
            }
        }
    }
}

async fn queue_missing_anibridge_scoped_id_hydration(app: &AppUseCase) {
    let mut pending_title_ids = std::collections::BTreeSet::new();

    match app
        .services
        .catalog
        .titles
        .list_anime_title_ids_missing_anibridge_scoped_external_ids(
            ANIBRIDGE_SCOPED_ID_BACKFILL_BATCH,
        )
        .await
    {
        Ok(title_ids) => {
            for title_id in title_ids {
                pending_title_ids.insert(title_id);
            }
        }
        Err(error) => {
            warn!(
                error = %error,
                "failed to queue anibridge scoped external ID hydration backfill"
            );
        }
    }

    match app
        .services
        .catalog
        .titles
        .list_anime_title_ids_missing_title_anidb_external_ids(ANIBRIDGE_SCOPED_ID_BACKFILL_BATCH)
        .await
    {
        Ok(title_ids) => {
            for title_id in title_ids {
                pending_title_ids.insert(title_id);
            }
        }
        Err(error) => {
            warn!(
                error = %error,
                "failed to queue title-level AniDB hydration backfill"
            );
        }
    }

    if pending_title_ids.is_empty() {
        return;
    }

    let mut queued = 0usize;
    for title_id in pending_title_ids {
        if app
            .services
            .catalog
            .titles
            .mark_title_metadata_hydration_due_now(&title_id)
            .await
            .is_ok()
        {
            queued += 1;
        }
    }
    info!(
        queued,
        "queued anime titles for anibridge/title AniDB hydration"
    );
    app.runtime.catalog.title_hydration_wake.notify_one();
}

async fn schedule_title_hydration_retry(
    app: &AppUseCase,
    title_id: &str,
    facet: &MediaFacet,
    previous_attempt_count: i64,
) {
    let Some((next_attempt_at, next_attempt_count)) =
        next_title_hydration_retry(chrono::Utc::now(), previous_attempt_count)
    else {
        metrics::counter!("scryer_title_metadata_hydration_terminal_failures_total").increment(1);
        warn!(
            hydration_source = HydrationSource::BackgroundDue.as_str(),
            facet = facet.as_str(),
            title_id = %title_id,
            max_attempts = TITLE_HYDRATION_MAX_ATTEMPTS,
            "title hydration loop: reached max retry attempts, clearing retry state"
        );
        let _ = app
            .services
            .catalog
            .titles
            .clear_title_metadata_hydration_retry_state(title_id)
            .await;
        return;
    };

    let _ = app
        .services
        .catalog
        .titles
        .schedule_title_metadata_hydration_retry(
            title_id,
            &next_attempt_at.to_rfc3339(),
            next_attempt_count,
        )
        .await;
}

fn next_title_hydration_retry(
    now: chrono::DateTime<chrono::Utc>,
    previous_attempt_count: i64,
) -> Option<(chrono::DateTime<chrono::Utc>, i64)> {
    let next_attempt_count = previous_attempt_count.saturating_add(1);
    if next_attempt_count >= TITLE_HYDRATION_MAX_ATTEMPTS {
        return None;
    }

    let retry_delay = title_hydration_retry_delay(next_attempt_count);
    let next_attempt_at = now
        + chrono::Duration::from_std(retry_delay)
            .unwrap_or_else(|_| chrono::Duration::seconds(300));
    Some((next_attempt_at, next_attempt_count))
}

fn title_hydration_retry_delay(attempt_count: i64) -> Duration {
    let exponent = attempt_count.saturating_sub(1).clamp(0, 30) as u32;
    let multiplier = 1u32.checked_shl(exponent).unwrap_or(u32::MAX);
    let delay = TITLE_HYDRATION_RETRY_BASE.saturating_mul(multiplier);
    delay.min(TITLE_HYDRATION_RETRY_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_title_hydration_retry_stops_after_max_attempts() {
        let now = chrono::Utc::now();
        assert!(next_title_hydration_retry(now, TITLE_HYDRATION_MAX_ATTEMPTS - 1).is_none());
    }

    #[test]
    fn next_title_hydration_retry_uses_backoff_and_clamps_to_max() {
        let now = chrono::Utc::now();
        let (next_attempt_at, next_attempt_count) =
            next_title_hydration_retry(now, 10).expect("retry should still schedule");
        assert_eq!(next_attempt_count, 11);
        assert_eq!(
            next_attempt_at - now,
            chrono::Duration::from_std(TITLE_HYDRATION_RETRY_MAX)
                .expect("chrono duration should convert")
        );
    }
}
