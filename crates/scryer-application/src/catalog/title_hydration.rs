use super::*;
use crate::catalog_workflow::{
    HYDRATION_BULK_BATCH_SIZE, HydrationSource, HydrationTarget, extract_tvdb_id,
};
use crate::polling_worker::PollingWorker;
use std::time::Duration;
use tracing::{debug, info, warn};

const TITLE_HYDRATION_MAX_BATCH: usize = HYDRATION_BULK_BATCH_SIZE;
const TITLE_HYDRATION_IDLE_POLL_INTERVAL: Duration = Duration::from_secs(30);
const TITLE_HYDRATION_STARTUP_JITTER_MAX: Duration = Duration::from_secs(10 * 60);
const TITLE_HYDRATION_BATCH_DELAY_MIN: Duration = Duration::from_secs(10);
const TITLE_HYDRATION_BATCH_DELAY_MAX: Duration = Duration::from_secs(30);
const TITLE_HYDRATION_RETRY_BASE: Duration = Duration::from_secs(10);
const TITLE_HYDRATION_RETRY_MAX: Duration = Duration::from_secs(300);
const TITLE_HYDRATION_MAX_ATTEMPTS: i64 = 12;

fn title_hydration_jitter_delay(
    seed: &str,
    stream: &str,
    minimum: Duration,
    maximum: Duration,
) -> Duration {
    debug_assert!(minimum <= maximum);
    let minimum_seconds = minimum.as_secs();
    let window_seconds = maximum
        .as_secs()
        .saturating_sub(minimum_seconds)
        .saturating_add(1);
    minimum
        + crate::scheduler::stable_jitter_offset(
            seed,
            "title_hydration",
            stream,
            Duration::from_secs(window_seconds),
        )
}

fn randomized_title_hydration_delay(
    stream: &str,
    minimum: Duration,
    maximum: Duration,
) -> Duration {
    title_hydration_jitter_delay(&uuid::Uuid::new_v4().to_string(), stream, minimum, maximum)
}

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
        startup_jitter_max_secs = TITLE_HYDRATION_STARTUP_JITTER_MAX.as_secs(),
        batch_delay_min_secs = TITLE_HYDRATION_BATCH_DELAY_MIN.as_secs(),
        batch_delay_max_secs = TITLE_HYDRATION_BATCH_DELAY_MAX.as_secs(),
        retry_base_secs = TITLE_HYDRATION_RETRY_BASE.as_secs(),
        retry_max_secs = TITLE_HYDRATION_RETRY_MAX.as_secs(),
        max_attempts = TITLE_HYDRATION_MAX_ATTEMPTS,
        "background title hydration loop started"
    );

    let startup_delay = randomized_title_hydration_delay(
        "startup",
        Duration::ZERO,
        TITLE_HYDRATION_STARTUP_JITTER_MAX,
    );
    if !startup_delay.is_zero() {
        info!(
            delay_secs = startup_delay.as_secs(),
            "title hydration loop: staggering initial backlog drain"
        );
        if !worker
            .wait_for_wake_or_timeout(&app.runtime.catalog.title_hydration_wake, startup_delay)
            .await
        {
            return;
        }
    }

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
            debug!(
                blocked_facets = ?active_scan_facet_labels(&blocked_facets),
                blocked_facets_before_dispatch =
                    ?active_scan_facet_labels(&blocked_facets_before_dispatch),
                "title hydration loop: active scan facets changed before dispatch"
            );
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
                if let Err(error) = app
                    .services
                    .catalog
                    .titles
                    .clear_title_metadata_hydration_retry_state(&due_title.title.id)
                    .await
                {
                    warn!(
                        hydration_source = HydrationSource::BackgroundDue.as_str(),
                        title_id = %due_title.title.id,
                        error = %error,
                        "title hydration loop: failed to clear retry state for title without tvdb id"
                    );
                }
                original_attempts.remove(&due_title.title.id);
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

        debug!(
            hydration_source = HydrationSource::BackgroundDue.as_str(),
            count = targets.len(),
            "title hydration loop: processing batch"
        );

        for _ in 0..targets.len() {
            metrics::counter!("scryer_title_metadata_hydration_attempts_total").increment(1);
        }

        let title_ids = targets
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
                    if let Err(error) = app
                        .services
                        .catalog
                        .titles
                        .mark_title_metadata_hydration_due_now(title_id)
                        .await
                    {
                        warn!(
                            hydration_source = HydrationSource::BackgroundDue.as_str(),
                            title_id = %title_id,
                            error = %error,
                            "title hydration loop: failed to keep unreported title due"
                        );
                    }
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

        let batch_delay = randomized_title_hydration_delay(
            "between_batches",
            TITLE_HYDRATION_BATCH_DELAY_MIN,
            TITLE_HYDRATION_BATCH_DELAY_MAX,
        );
        debug!(
            delay_secs = batch_delay.as_secs(),
            "title hydration loop: pacing next background batch"
        );
        if !worker.wait_for_sleep(batch_delay).await {
            return;
        }
    }
}

async fn schedule_title_hydration_retry(
    app: &AppUseCase,
    title_id: &str,
    facet: &MediaFacet,
    previous_attempt_count: i64,
) {
    let Some((next_attempt_at, next_attempt_count)) =
        next_title_hydration_retry(app.runtime.environment.now(), previous_attempt_count)
    else {
        metrics::counter!("scryer_title_metadata_hydration_terminal_failures_total").increment(1);
        warn!(
            hydration_source = HydrationSource::BackgroundDue.as_str(),
            facet = facet.as_str(),
            title_id = %title_id,
            max_attempts = TITLE_HYDRATION_MAX_ATTEMPTS,
            "title hydration loop: reached max retry attempts, clearing retry state"
        );
        if let Err(error) = app
            .services
            .catalog
            .titles
            .clear_title_metadata_hydration_retry_state(title_id)
            .await
        {
            warn!(
                hydration_source = HydrationSource::BackgroundDue.as_str(),
                facet = facet.as_str(),
                title_id = %title_id,
                error = %error,
                "title hydration loop: failed to clear terminal retry state"
            );
        }
        return;
    };

    if let Err(error) = app
        .services
        .catalog
        .titles
        .schedule_title_metadata_hydration_retry(
            title_id,
            &next_attempt_at.to_rfc3339(),
            next_attempt_count,
        )
        .await
    {
        warn!(
            hydration_source = HydrationSource::BackgroundDue.as_str(),
            facet = facet.as_str(),
            title_id = %title_id,
            attempt_count = next_attempt_count,
            next_attempt_at = %next_attempt_at,
            error = %error,
            "title hydration loop: failed to schedule retry"
        );
    }
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
    fn background_batch_jitter_stays_between_ten_and_thirty_seconds() {
        assert_eq!(TITLE_HYDRATION_BATCH_DELAY_MIN, Duration::from_secs(10));
        assert_eq!(TITLE_HYDRATION_BATCH_DELAY_MAX, Duration::from_secs(30));
        for seed in ["instance-a", "instance-b", "instance-c", "instance-d"] {
            let delay = title_hydration_jitter_delay(
                seed,
                "between_batches",
                TITLE_HYDRATION_BATCH_DELAY_MIN,
                TITLE_HYDRATION_BATCH_DELAY_MAX,
            );
            assert!(
                (TITLE_HYDRATION_BATCH_DELAY_MIN..=TITLE_HYDRATION_BATCH_DELAY_MAX)
                    .contains(&delay)
            );
        }
    }

    #[test]
    fn startup_jitter_is_ephemeral_and_bounded() {
        let delay = title_hydration_jitter_delay(
            "ephemeral-process-seed",
            "startup",
            Duration::ZERO,
            TITLE_HYDRATION_STARTUP_JITTER_MAX,
        );
        assert!(delay <= TITLE_HYDRATION_STARTUP_JITTER_MAX);
    }

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
