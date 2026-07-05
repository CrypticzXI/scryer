use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AccountQuotaKey, AdmissionReason, AppResult, DeferralReason, RateLimitCooldownAction,
    RssFreshnessContext, SchedulerAdmission, SchedulerBatchDecision, SchedulerBatchRequest,
    SchedulerCandidate, SchedulerFeedback, SchedulerFeedbackOutcome, SchedulerIntent,
    SchedulerLease, SchedulerOperation, SchedulerSnapshot, SchedulerSnapshotEntry,
    SchedulerSnapshotFilter, SkipReason, UpstreamScheduler,
};
use scryer_outbound_http::PersistedDestinationCooldown;
use scryer_outbound_http::{DestinationKey, HostKey, RateLimitRegistry, RetryAfterSource};
use tracing::warn;
use uuid::Uuid;

use crate::queries::sql_runtime::{SqlArg, SqlRow, SqlRuntime, StoreDatastore};

const LOW_VALUE_BACKGROUND_THRESHOLD: f64 = 0.25;
const LOW_VALUE_SUBTITLE_THRESHOLD: f64 = 0.15;
const RSS_FRESHNESS_ESCALATION_THRESHOLD: f64 = 0.85;
const LOW_ACCOUNT_QUOTA_REMAINING_FRACTION: f64 = 0.20;
const DEFAULT_RSS_TARGET_INTERVAL: Duration = Duration::from_secs(15 * 60);
const LOW_QUOTA_RSS_TARGET_INTERVAL: Duration = Duration::from_secs(60 * 60);
const QUOTA_OBSERVATION_STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const EXHAUSTED_QUOTA_PROBE_AFTER: Duration = Duration::from_secs(6 * 60 * 60);
const SCHEDULER_STATE_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const RATE_LIMIT_FALLBACK_COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Clone, Default)]
pub struct InMemoryUpstreamScheduler {
    state: Arc<Mutex<SchedulerState>>,
    persistence: Option<Arc<SqlUpstreamSchedulerStore>>,
}

#[derive(Default)]
struct SchedulerState {
    entries: HashMap<SchedulerStateKey, SchedulerStateEntry>,
    rss_cadence: HashMap<SchedulerStateKey, RssCadenceEntry>,
    dirty: HashSet<SchedulerStateKey>,
    dirty_rss_cadence: HashSet<SchedulerStateKey>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SchedulerStateKey {
    host_key: HostKey,
    destination_key: DestinationKey,
    account_quota_key: Option<AccountQuotaKey>,
    rss_request_key: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct SchedulerStateEntry {
    last_decision: Option<String>,
    last_feedback_at: Option<DateTime<Utc>>,
    last_successful_at: Option<DateTime<Utc>>,
    last_attempt_at: Option<DateTime<Utc>>,
    api_current: Option<u64>,
    api_max: Option<u64>,
    grab_current: Option<u64>,
    grab_max: Option<u64>,
    quota_observed_at: Option<DateTime<Utc>>,
    quota_probe_after: Option<DateTime<Utc>>,
    quota_reset_at: Option<DateTime<Utc>>,
    quota_source: Option<String>,
    admitted_count: u64,
    deferred_count: u64,
    skipped_count: u64,
}

#[derive(Clone, Default)]
struct RssCadenceEntry {
    last_successful_poll_at: Option<DateTime<Utc>>,
    last_attempt_at: Option<DateTime<Utc>>,
    target_interval: Option<Duration>,
    latest_safe_poll_at: Option<DateTime<Utc>>,
    estimated_feed_depth: Option<u32>,
    freshness_risk: f64,
    destination_recent_activity_at: Option<DateTime<Utc>>,
    last_seen_release_identity: Option<String>,
    last_seen_release_published_at: Option<DateTime<Utc>>,
    last_feed_gap_start_at: Option<DateTime<Utc>>,
    last_feed_gap_end_at: Option<DateTime<Utc>>,
}

impl InMemoryUpstreamScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn new_persistent(datastore: StoreDatastore) -> AppResult<Self> {
        let store = Arc::new(SqlUpstreamSchedulerStore::new(datastore));
        store.prune_stale_rows(Utc::now()).await?;
        let entries = store.load_entries().await?;
        let rss_cadence = store.load_rss_cadence().await?;
        let destination_cooldowns = store.load_destination_cooldowns().await?;
        RateLimitRegistry::new().hydrate_destination_cooldowns(destination_cooldowns);
        let scheduler = Self {
            state: Arc::new(Mutex::new(SchedulerState {
                entries,
                rss_cadence,
                dirty: HashSet::new(),
                dirty_rss_cadence: HashSet::new(),
            })),
            persistence: Some(store),
        };
        scheduler.spawn_flush_task();
        Ok(scheduler)
    }

    fn spawn_flush_task(&self) {
        let Some(store) = self.persistence.clone() else {
            return;
        };
        let state = self.state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            loop {
                interval.tick().await;
                if let Err(error) = flush_dirty_state(&store, &state).await {
                    warn!(error = %error, "failed to flush upstream scheduler state");
                }
            }
        });
    }

    pub async fn flush_pending(&self) -> AppResult<()> {
        let Some(store) = self.persistence.clone() else {
            return Ok(());
        };
        flush_dirty_state(&store, &self.state).await
    }
}

#[async_trait]
impl UpstreamScheduler for InMemoryUpstreamScheduler {
    async fn admit_batch(
        &self,
        request: SchedulerBatchRequest,
    ) -> AppResult<SchedulerBatchDecision> {
        let mut scored = request
            .candidates
            .into_iter()
            .map(|candidate| {
                let score = candidate_score(&candidate, request.now);
                (candidate, score)
            })
            .collect::<Vec<_>>();
        scored.sort_by(|(_, left), (_, right)| {
            right.partial_cmp(left).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut decisions = Vec::with_capacity(scored.len());
        {
            let mut state = self.state.lock().expect("upstream scheduler lock poisoned");
            for (candidate, _) in scored {
                let key = candidate_state_key(&candidate);
                let operation = candidate.operation;
                let intent = candidate.intent;
                let freshness = candidate.freshness.clone();
                let decision = decide_candidate(candidate, request.now, state.entries.get(&key));
                record_decision_in_state(
                    &mut state,
                    key,
                    &decision,
                    operation,
                    intent,
                    freshness.as_ref(),
                );
                decisions.push(decision);
            }
        }

        Ok(SchedulerBatchDecision {
            batch_id: request.batch_id,
            decisions,
        })
    }

    async fn record_feedback(&self, feedback: SchedulerFeedback) -> AppResult<()> {
        let is_rss_feedback = feedback.lease.as_ref().is_some_and(|lease| {
            lease.operation == SchedulerOperation::Rss
                || lease.intent == SchedulerIntent::BackgroundRss
        });
        let key = SchedulerStateKey {
            host_key: feedback.host_key.clone(),
            destination_key: feedback.destination_key.clone(),
            account_quota_key: feedback.account_quota_key.clone(),
            rss_request_key: feedback
                .lease
                .as_ref()
                .and_then(|lease| lease.rss_request_key.clone()),
        };
        let cooldown_record = if matches!(feedback.outcome, SchedulerFeedbackOutcome::RateLimited)
            && feedback.cooldown_action == RateLimitCooldownAction::RecordFallback
        {
            match feedback.retry_after.filter(|delay| !delay.is_zero()) {
                Some(delay) => Some((delay, RetryAfterSource::Seconds)),
                None => Some((
                    RATE_LIMIT_FALLBACK_COOLDOWN,
                    RetryAfterSource::FallbackBackoff,
                )),
            }
        } else {
            None
        };
        {
            let mut state = self.state.lock().expect("upstream scheduler lock poisoned");
            let entry = state.entries.entry(key.clone()).or_default();
            entry.last_feedback_at = Some(feedback.observed_at);
            entry.last_attempt_at = Some(feedback.observed_at);
            if matches!(
                feedback.outcome,
                SchedulerFeedbackOutcome::Success | SchedulerFeedbackOutcome::EmptySuccess
            ) {
                entry.last_successful_at = Some(feedback.observed_at);
            }
            apply_quota_observation(entry, &feedback);
            entry.last_decision = Some(
                match feedback.outcome {
                    SchedulerFeedbackOutcome::Success => "feedback:success",
                    SchedulerFeedbackOutcome::EmptySuccess => "feedback:empty_success",
                    SchedulerFeedbackOutcome::RateLimited => "feedback:rate_limited",
                    SchedulerFeedbackOutcome::TransportFailure => "feedback:transport_failure",
                    SchedulerFeedbackOutcome::ProviderFailure => "feedback:provider_failure",
                    SchedulerFeedbackOutcome::Cancelled => "feedback:cancelled",
                }
                .to_string(),
            );
            let quota_is_fresh = !quota_is_stale(entry, feedback.observed_at);
            let quota_exhausted = quota_is_fresh && observed_quota_exhausted(entry);
            let api_remaining = quota_is_fresh
                .then(|| api_remaining_fraction(entry))
                .flatten();
            state.dirty.insert(key.clone());
            if is_rss_feedback {
                let cadence = state.rss_cadence.entry(key.clone()).or_default();
                cadence.last_attempt_at = Some(feedback.observed_at);
                cadence.target_interval = Some(rss_target_interval_for_quota(
                    api_remaining,
                    quota_exhausted,
                ));
                if matches!(
                    feedback.outcome,
                    SchedulerFeedbackOutcome::Success | SchedulerFeedbackOutcome::EmptySuccess
                ) {
                    cadence.last_successful_poll_at = Some(feedback.observed_at);
                    cadence.estimated_feed_depth = feedback.rss_feed_result_count;
                    if let Some(previous_identity) = cadence.last_seen_release_identity.as_ref()
                        && !feedback.rss_seen_release_identities.is_empty()
                        && !feedback
                            .rss_seen_release_identities
                            .iter()
                            .any(|identity| identity == previous_identity)
                    {
                        cadence.last_feed_gap_start_at = cadence
                            .last_seen_release_published_at
                            .or(cadence.last_successful_poll_at);
                        cadence.last_feed_gap_end_at = feedback
                            .rss_last_seen_release_published_at
                            .or(Some(feedback.observed_at));
                    }
                    if let Some(identity) = feedback.rss_last_seen_release_identity.clone() {
                        cadence.last_seen_release_identity = Some(identity);
                    }
                    if let Some(published_at) = feedback.rss_last_seen_release_published_at {
                        cadence.last_seen_release_published_at = Some(published_at);
                    }
                }
                let target_interval = cadence
                    .target_interval
                    .unwrap_or(DEFAULT_RSS_TARGET_INTERVAL);
                cadence.latest_safe_poll_at = chrono::Duration::from_std(target_interval)
                    .ok()
                    .map(|duration| feedback.observed_at + duration);
                cadence.freshness_risk = match feedback.outcome {
                    SchedulerFeedbackOutcome::Success | SchedulerFeedbackOutcome::EmptySuccess => {
                        0.0
                    }
                    SchedulerFeedbackOutcome::RateLimited => 0.5,
                    SchedulerFeedbackOutcome::TransportFailure
                    | SchedulerFeedbackOutcome::ProviderFailure => 0.75,
                    SchedulerFeedbackOutcome::Cancelled => cadence.freshness_risk,
                };
                cadence.destination_recent_activity_at = Some(feedback.observed_at);
                state.dirty_rss_cadence.insert(key.clone());
            }
        }
        if let Some((delay, source)) = cooldown_record {
            let _ = RateLimitRegistry::new()
                .record_destination_cooldown(&key.destination_key, delay, source)
                .await;
        }
        Ok(())
    }

    async fn snapshot(&self, filter: SchedulerSnapshotFilter) -> AppResult<SchedulerSnapshot> {
        let state = self.state.lock().expect("upstream scheduler lock poisoned");
        let entries = state
            .entries
            .iter()
            .filter(|(key, _)| {
                filter
                    .host_key
                    .as_ref()
                    .is_none_or(|host| host == &key.host_key)
                    && filter
                        .destination_key
                        .as_ref()
                        .is_none_or(|destination| destination == &key.destination_key)
                    && filter
                        .account_quota_key
                        .as_ref()
                        .is_none_or(|account| Some(account) == key.account_quota_key.as_ref())
            })
            .map(|(key, entry)| {
                let cadence = state.rss_cadence.get(key);
                SchedulerSnapshotEntry {
                    host_key: key.host_key.clone(),
                    destination_key: key.destination_key.clone(),
                    account_quota_key: key.account_quota_key.clone(),
                    rss_request_key: key.rss_request_key.clone(),
                    last_decision: entry.last_decision.clone(),
                    last_feedback_at: entry.last_feedback_at,
                    last_successful_at: entry.last_successful_at,
                    last_attempt_at: entry.last_attempt_at,
                    cooldown_until: destination_cooldown_until(&key.destination_key, Utc::now()),
                    api_remaining_fraction: api_remaining_fraction(entry),
                    quota_observed_at: entry.quota_observed_at,
                    quota_probe_after: entry.quota_probe_after,
                    quota_reset_at: entry.quota_reset_at,
                    quota_source: entry.quota_source.clone(),
                    quota_stale: quota_is_stale(entry, Utc::now()),
                    rss_last_successful_poll_at: cadence
                        .and_then(|entry| entry.last_successful_poll_at),
                    rss_last_attempt_at: cadence.and_then(|entry| entry.last_attempt_at),
                    rss_target_interval: cadence.and_then(|entry| entry.target_interval),
                    rss_latest_safe_poll_at: cadence.and_then(|entry| entry.latest_safe_poll_at),
                    rss_estimated_feed_depth: cadence.and_then(|entry| entry.estimated_feed_depth),
                    rss_freshness_risk: cadence.map(|entry| entry.freshness_risk),
                    rss_destination_recent_activity_at: cadence
                        .and_then(|entry| entry.destination_recent_activity_at),
                    rss_last_seen_release_identity: cadence
                        .and_then(|entry| entry.last_seen_release_identity.clone()),
                    rss_last_seen_release_published_at: cadence
                        .and_then(|entry| entry.last_seen_release_published_at),
                    rss_last_feed_gap_start_at: cadence
                        .and_then(|entry| entry.last_feed_gap_start_at),
                    rss_last_feed_gap_end_at: cadence.and_then(|entry| entry.last_feed_gap_end_at),
                    admitted_count: entry.admitted_count,
                    deferred_count: entry.deferred_count,
                    skipped_count: entry.skipped_count,
                }
            })
            .collect();

        Ok(SchedulerSnapshot { entries })
    }

    async fn flush_pending(&self) -> AppResult<()> {
        InMemoryUpstreamScheduler::flush_pending(self).await
    }
}

fn record_decision_in_state(
    state: &mut SchedulerState,
    key: SchedulerStateKey,
    decision: &SchedulerAdmission,
    operation: SchedulerOperation,
    intent: SchedulerIntent,
    freshness: Option<&RssFreshnessContext>,
) {
    let label = decision_label(decision);
    let entry = state.entries.entry(key.clone()).or_default();
    entry.last_decision = Some(label.to_string());
    match decision {
        SchedulerAdmission::Admit { .. } => entry.admitted_count += 1,
        SchedulerAdmission::Defer { .. } => entry.deferred_count += 1,
        SchedulerAdmission::Skip { .. } => entry.skipped_count += 1,
    }
    state.dirty.insert(key.clone());
    if matches!(decision, SchedulerAdmission::Defer { .. })
        && operation == SchedulerOperation::Rss
        && intent == SchedulerIntent::BackgroundRss
        && let Some(freshness) = freshness
    {
        let cadence = state.rss_cadence.entry(key.clone()).or_default();
        cadence.target_interval = Some(freshness.target_interval);
        cadence.latest_safe_poll_at = Some(freshness.latest_safe_poll_at);
        cadence.estimated_feed_depth = freshness.estimated_feed_depth;
        cadence.freshness_risk = freshness.freshness_risk;
        cadence.destination_recent_activity_at = freshness.destination_recent_activity_at;
        state.dirty_rss_cadence.insert(key);
    }
}

fn apply_quota_observation(entry: &mut SchedulerStateEntry, feedback: &SchedulerFeedback) {
    let observed_quota = feedback.observed_api_current.is_some()
        || feedback.observed_api_max.is_some()
        || feedback.observed_grab_current.is_some()
        || feedback.observed_grab_max.is_some();
    if !observed_quota {
        return;
    }

    if entry
        .quota_observed_at
        .is_some_and(|observed_at| feedback.observed_at < observed_at)
    {
        return;
    }

    if let Some(value) = feedback.observed_api_current {
        entry.api_current = Some(value);
    }
    if let Some(value) = feedback.observed_api_max {
        entry.api_max = Some(value);
    }
    if let Some(value) = feedback.observed_grab_current {
        entry.grab_current = Some(value);
    }
    if let Some(value) = feedback.observed_grab_max {
        entry.grab_max = Some(value);
    }
    entry.quota_observed_at = Some(feedback.observed_at);
    entry.quota_probe_after = observed_quota_exhausted(entry)
        .then(|| chrono::Duration::from_std(EXHAUSTED_QUOTA_PROBE_AFTER).ok())
        .flatten()
        .map(|delay| feedback.observed_at + delay);
    entry.quota_source = Some("scheduler_feedback".to_string());
}

#[derive(Clone)]
struct SqlUpstreamSchedulerStore {
    datastore: StoreDatastore,
}

impl SqlUpstreamSchedulerStore {
    fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    async fn prune_stale_rows(&self, now: DateTime<Utc>) -> AppResult<()> {
        let retention = chrono::Duration::from_std(SCHEDULER_STATE_RETENTION)
            .unwrap_or_else(|_| chrono::Duration::days(30));
        let state_cutoff = now - retention;
        SqlRuntime::execute(
            self.datastore.read_exec(),
            "DELETE FROM upstream_destination_cooldowns
             WHERE cooldown_until <= {}",
            &[SqlArg::Timestamp(now)],
        )
        .await?;
        SqlRuntime::execute(
            self.datastore.read_exec(),
            "DELETE FROM upstream_scheduler_states
             WHERE updated_at < {}
               AND NOT EXISTS (
                   SELECT 1
                   FROM upstream_destination_cooldowns
                   WHERE upstream_destination_cooldowns.destination_key = upstream_scheduler_states.destination_key
                     AND upstream_destination_cooldowns.cooldown_until > {}
               )",
            &[SqlArg::Timestamp(state_cutoff), SqlArg::Timestamp(now)],
        )
        .await?;
        SqlRuntime::execute(
            self.datastore.read_exec(),
            "DELETE FROM upstream_scheduler_rss_cadence
             WHERE updated_at < {}
               AND (latest_safe_poll_at IS NULL OR latest_safe_poll_at < {})
               AND NOT EXISTS (
                   SELECT 1
                   FROM upstream_destination_cooldowns
                   WHERE upstream_destination_cooldowns.destination_key = upstream_scheduler_rss_cadence.destination_key
                     AND upstream_destination_cooldowns.cooldown_until > {}
               )",
            &[
                SqlArg::Timestamp(state_cutoff),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
        Ok(())
    }

    async fn load_entries(&self) -> AppResult<HashMap<SchedulerStateKey, SchedulerStateEntry>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT host_key, destination_key, account_quota_key, rss_request_key,
                    api_current, api_max, grab_current, grab_max, quota_observed_at,
                    quota_probe_after, quota_reset_at, quota_source, last_decision,
                    last_feedback_at, last_successful_at, last_attempt_at,
                    admitted_count, deferred_count, skipped_count
             FROM upstream_scheduler_states",
            &[],
        )
        .await?;

        let mut entries = HashMap::new();
        for row in rows {
            let (key, entry) = row_to_scheduler_entry(&row)?;
            entries.insert(key, entry);
        }
        Ok(entries)
    }

    async fn load_rss_cadence(&self) -> AppResult<HashMap<SchedulerStateKey, RssCadenceEntry>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT host_key, destination_key, account_quota_key, rss_request_key,
                    last_successful_poll_at, last_attempt_at, target_interval_seconds,
                    latest_safe_poll_at, estimated_feed_depth, freshness_risk,
                    destination_recent_activity_at, last_seen_release_identity,
                    last_seen_release_published_at, last_feed_gap_start_at,
                    last_feed_gap_end_at
             FROM upstream_scheduler_rss_cadence",
            &[],
        )
        .await?;

        let mut entries = HashMap::new();
        for row in rows {
            let account_quota_key = row.text("account_quota_key")?.trim().to_string();
            let rss_request_key = row
                .opt_text("rss_request_key")?
                .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_string()));
            let key = SchedulerStateKey {
                host_key: HostKey::from(row.text("host_key")?),
                destination_key: DestinationKey::from(row.text("destination_key")?),
                account_quota_key: (!account_quota_key.is_empty())
                    .then(|| AccountQuotaKey::from(account_quota_key)),
                rss_request_key,
            };
            let target_interval = row
                .opt_i64("target_interval_seconds")?
                .and_then(|seconds| (seconds > 0).then(|| Duration::from_secs(seconds as u64)));
            let estimated_feed_depth = row
                .opt_i64("estimated_feed_depth")?
                .map(|value| value.max(0).min(u32::MAX as i64) as u32);
            entries.insert(
                key,
                RssCadenceEntry {
                    last_successful_poll_at: row.opt_timestamp("last_successful_poll_at")?,
                    last_attempt_at: row.opt_timestamp("last_attempt_at")?,
                    target_interval,
                    latest_safe_poll_at: row.opt_timestamp("latest_safe_poll_at")?,
                    estimated_feed_depth,
                    freshness_risk: row.opt_f64("freshness_risk")?.unwrap_or_default(),
                    destination_recent_activity_at: row
                        .opt_timestamp("destination_recent_activity_at")?,
                    last_seen_release_identity: row.opt_text("last_seen_release_identity")?,
                    last_seen_release_published_at: row
                        .opt_timestamp("last_seen_release_published_at")?,
                    last_feed_gap_start_at: row.opt_timestamp("last_feed_gap_start_at")?,
                    last_feed_gap_end_at: row.opt_timestamp("last_feed_gap_end_at")?,
                },
            );
        }
        Ok(entries)
    }

    async fn load_destination_cooldowns(&self) -> AppResult<Vec<PersistedDestinationCooldown>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT destination_key, cooldown_until, retry_after_seconds, source,
                    status_code, message, observed_at
             FROM upstream_destination_cooldowns",
            &[],
        )
        .await?;

        let mut cooldowns = Vec::new();
        for row in rows {
            let source = RetryAfterSource::from_persistent_str(&row.text("source")?)
                .unwrap_or(RetryAfterSource::ExistingCooldown);
            let retry_after = row
                .opt_i64("retry_after_seconds")?
                .and_then(|seconds| (seconds > 0).then(|| Duration::from_secs(seconds as u64)));
            let status_code = row
                .opt_i64("status_code")?
                .and_then(|value| u16::try_from(value).ok());
            cooldowns.push(PersistedDestinationCooldown {
                destination_key: DestinationKey::from(row.text("destination_key")?),
                cooldown_until: row
                    .opt_timestamp("cooldown_until")?
                    .unwrap_or_else(Utc::now),
                retry_after,
                source,
                status_code,
                message: row.opt_text("message")?,
                observed_at: row.opt_timestamp("observed_at")?.unwrap_or_else(Utc::now),
            });
        }
        Ok(cooldowns)
    }

    async fn flush_entries(
        &self,
        entries: Vec<(SchedulerStateKey, SchedulerStateEntry)>,
    ) -> AppResult<()> {
        if entries.is_empty() {
            return Ok(());
        }

        for (key, entry) in entries {
            SqlRuntime::execute(
                self.datastore.read_exec(),
                "INSERT INTO upstream_scheduler_states (
                    host_key, destination_key, account_quota_key, rss_request_key,
                    api_current, api_max, grab_current, grab_max, quota_observed_at,
                    quota_probe_after, quota_reset_at, quota_source, last_decision,
                    last_feedback_at, last_successful_at, last_attempt_at, admitted_count,
                    deferred_count, skipped_count, updated_at
                 )
                 VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                 ON CONFLICT (host_key, destination_key, account_quota_key, rss_request_key)
                 DO UPDATE SET
                    api_current = CASE
                        WHEN excluded.quota_observed_at IS NOT NULL THEN excluded.api_current
                        ELSE upstream_scheduler_states.api_current
                    END,
                    api_max = CASE
                        WHEN excluded.quota_observed_at IS NOT NULL THEN excluded.api_max
                        ELSE upstream_scheduler_states.api_max
                    END,
                    grab_current = CASE
                        WHEN excluded.quota_observed_at IS NOT NULL THEN excluded.grab_current
                        ELSE upstream_scheduler_states.grab_current
                    END,
                    grab_max = CASE
                        WHEN excluded.quota_observed_at IS NOT NULL THEN excluded.grab_max
                        ELSE upstream_scheduler_states.grab_max
                    END,
                    quota_observed_at = COALESCE(excluded.quota_observed_at, upstream_scheduler_states.quota_observed_at),
                    quota_probe_after = COALESCE(excluded.quota_probe_after, upstream_scheduler_states.quota_probe_after),
                    quota_reset_at = COALESCE(excluded.quota_reset_at, upstream_scheduler_states.quota_reset_at),
                    quota_source = COALESCE(excluded.quota_source, upstream_scheduler_states.quota_source),
                    last_decision = COALESCE(excluded.last_decision, upstream_scheduler_states.last_decision),
                    last_feedback_at = COALESCE(excluded.last_feedback_at, upstream_scheduler_states.last_feedback_at),
                    last_successful_at = COALESCE(excluded.last_successful_at, upstream_scheduler_states.last_successful_at),
                    last_attempt_at = COALESCE(excluded.last_attempt_at, upstream_scheduler_states.last_attempt_at),
                    admitted_count = excluded.admitted_count,
                    deferred_count = excluded.deferred_count,
                    skipped_count = excluded.skipped_count,
                    updated_at = excluded.updated_at",
                &[
                    SqlArg::Text(key.host_key.to_string()),
                    SqlArg::Text(key.destination_key.to_string()),
                    SqlArg::Text(
                        key.account_quota_key
                            .as_ref()
                            .map(ToString::to_string)
                            .unwrap_or_default(),
                    ),
                    SqlArg::Text(key.rss_request_key.clone().unwrap_or_default()),
                    SqlArg::OptI64(entry.api_current.map(|value| value as i64)),
                    SqlArg::OptI64(entry.api_max.map(|value| value as i64)),
                    SqlArg::OptI64(entry.grab_current.map(|value| value as i64)),
                    SqlArg::OptI64(entry.grab_max.map(|value| value as i64)),
                    SqlArg::OptTimestamp(entry.quota_observed_at),
                    SqlArg::OptTimestamp(entry.quota_probe_after),
                    SqlArg::OptTimestamp(entry.quota_reset_at),
                    SqlArg::OptText(entry.quota_source),
                    SqlArg::OptText(entry.last_decision),
                    SqlArg::OptTimestamp(entry.last_feedback_at),
                    SqlArg::OptTimestamp(entry.last_successful_at),
                    SqlArg::OptTimestamp(entry.last_attempt_at),
                    SqlArg::I64(entry.admitted_count as i64),
                    SqlArg::I64(entry.deferred_count as i64),
                    SqlArg::I64(entry.skipped_count as i64),
                    SqlArg::Timestamp(Utc::now()),
                ],
            )
            .await?;
        }
        Ok(())
    }

    async fn flush_rss_cadence(
        &self,
        entries: Vec<(SchedulerStateKey, RssCadenceEntry)>,
    ) -> AppResult<()> {
        if entries.is_empty() {
            return Ok(());
        }

        for (key, entry) in entries {
            let target_interval_seconds = entry
                .target_interval
                .unwrap_or_else(|| Duration::from_secs(15 * 60))
                .as_secs()
                .min(i64::MAX as u64) as i64;
            SqlRuntime::execute(
                self.datastore.read_exec(),
                "INSERT INTO upstream_scheduler_rss_cadence (
                    host_key, destination_key, account_quota_key, rss_request_key,
                    last_successful_poll_at, last_attempt_at, target_interval_seconds,
                    latest_safe_poll_at, estimated_feed_depth, freshness_risk,
                    destination_recent_activity_at, last_seen_release_identity,
                    last_seen_release_published_at, last_feed_gap_start_at,
                    last_feed_gap_end_at, updated_at
                 )
                 VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                 ON CONFLICT (host_key, destination_key, account_quota_key, rss_request_key)
                 DO UPDATE SET
                    last_successful_poll_at = COALESCE(excluded.last_successful_poll_at, upstream_scheduler_rss_cadence.last_successful_poll_at),
                    last_attempt_at = COALESCE(excluded.last_attempt_at, upstream_scheduler_rss_cadence.last_attempt_at),
                    target_interval_seconds = excluded.target_interval_seconds,
                    latest_safe_poll_at = COALESCE(excluded.latest_safe_poll_at, upstream_scheduler_rss_cadence.latest_safe_poll_at),
                    estimated_feed_depth = COALESCE(excluded.estimated_feed_depth, upstream_scheduler_rss_cadence.estimated_feed_depth),
                    freshness_risk = excluded.freshness_risk,
                    destination_recent_activity_at = COALESCE(excluded.destination_recent_activity_at, upstream_scheduler_rss_cadence.destination_recent_activity_at),
                    last_seen_release_identity = COALESCE(excluded.last_seen_release_identity, upstream_scheduler_rss_cadence.last_seen_release_identity),
                    last_seen_release_published_at = COALESCE(excluded.last_seen_release_published_at, upstream_scheduler_rss_cadence.last_seen_release_published_at),
                    last_feed_gap_start_at = COALESCE(excluded.last_feed_gap_start_at, upstream_scheduler_rss_cadence.last_feed_gap_start_at),
                    last_feed_gap_end_at = COALESCE(excluded.last_feed_gap_end_at, upstream_scheduler_rss_cadence.last_feed_gap_end_at),
                    updated_at = excluded.updated_at",
                &[
                    SqlArg::Text(key.host_key.to_string()),
                    SqlArg::Text(key.destination_key.to_string()),
                    SqlArg::Text(
                        key.account_quota_key
                            .as_ref()
                            .map(ToString::to_string)
                            .unwrap_or_default(),
                    ),
                    SqlArg::Text(key.rss_request_key.clone().unwrap_or_default()),
                    SqlArg::OptTimestamp(entry.last_successful_poll_at),
                    SqlArg::OptTimestamp(entry.last_attempt_at),
                    SqlArg::I64(target_interval_seconds),
                    SqlArg::OptTimestamp(entry.latest_safe_poll_at),
                    SqlArg::OptI64(entry.estimated_feed_depth.map(i64::from)),
                    SqlArg::F64(entry.freshness_risk),
                    SqlArg::OptTimestamp(entry.destination_recent_activity_at),
                    SqlArg::OptText(entry.last_seen_release_identity),
                    SqlArg::OptTimestamp(entry.last_seen_release_published_at),
                    SqlArg::OptTimestamp(entry.last_feed_gap_start_at),
                    SqlArg::OptTimestamp(entry.last_feed_gap_end_at),
                    SqlArg::Timestamp(Utc::now()),
                ],
            )
            .await?;
        }
        Ok(())
    }

    async fn flush_destination_cooldowns(
        &self,
        cooldowns: Vec<PersistedDestinationCooldown>,
    ) -> AppResult<()> {
        if cooldowns.is_empty() {
            return Ok(());
        }

        for cooldown in cooldowns {
            let retry_after_seconds = cooldown
                .retry_after
                .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64);
            SqlRuntime::execute(
                self.datastore.read_exec(),
                "INSERT INTO upstream_destination_cooldowns (
                    destination_key, cooldown_until, retry_after_seconds, source,
                    status_code, message, observed_at, updated_at
                 )
                 VALUES ({}, {}, {}, {}, {}, {}, {}, {})
                 ON CONFLICT (destination_key)
                 DO UPDATE SET
                    cooldown_until = excluded.cooldown_until,
                    retry_after_seconds = excluded.retry_after_seconds,
                    source = excluded.source,
                    status_code = excluded.status_code,
                    message = excluded.message,
                    observed_at = excluded.observed_at,
                    updated_at = excluded.updated_at",
                &[
                    SqlArg::Text(cooldown.destination_key.to_string()),
                    SqlArg::Timestamp(cooldown.cooldown_until),
                    SqlArg::OptI64(retry_after_seconds),
                    SqlArg::Text(cooldown.source.as_persistent_str().to_string()),
                    SqlArg::OptI64(cooldown.status_code.map(i64::from)),
                    SqlArg::OptText(cooldown.message),
                    SqlArg::Timestamp(cooldown.observed_at),
                    SqlArg::Timestamp(Utc::now()),
                ],
            )
            .await?;
        }
        Ok(())
    }
}

fn drain_dirty_state(
    state: &Arc<Mutex<SchedulerState>>,
) -> (
    Vec<(SchedulerStateKey, SchedulerStateEntry)>,
    Vec<(SchedulerStateKey, RssCadenceEntry)>,
) {
    let mut state = state.lock().expect("upstream scheduler lock poisoned");
    let dirty = std::mem::take(&mut state.dirty);
    let dirty_rss = std::mem::take(&mut state.dirty_rss_cadence);
    let entries = dirty
        .into_iter()
        .filter_map(|key| state.entries.get(&key).cloned().map(|entry| (key, entry)))
        .collect();
    let rss_cadence = dirty_rss
        .into_iter()
        .filter_map(|key| {
            state
                .rss_cadence
                .get(&key)
                .cloned()
                .map(|entry| (key, entry))
        })
        .collect();
    (entries, rss_cadence)
}

async fn flush_dirty_state(
    store: &SqlUpstreamSchedulerStore,
    state: &Arc<Mutex<SchedulerState>>,
) -> AppResult<()> {
    let (entries, rss_cadence) = drain_dirty_state(state);
    let destination_cooldowns = RateLimitRegistry::new().drain_dirty_destination_cooldowns();
    if entries.is_empty() && rss_cadence.is_empty() && destination_cooldowns.is_empty() {
        return Ok(());
    }

    let entry_keys = entries
        .iter()
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    let rss_keys = rss_cadence
        .iter()
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();

    if let Err(error) = store.flush_entries(entries).await {
        requeue_dirty_entries(state, entry_keys);
        requeue_dirty_rss_cadence(state, rss_keys);
        RateLimitRegistry::new().requeue_dirty_destination_cooldowns(destination_cooldowns);
        return Err(error);
    }
    if let Err(error) = store.flush_rss_cadence(rss_cadence).await {
        requeue_dirty_rss_cadence(state, rss_keys);
        RateLimitRegistry::new().requeue_dirty_destination_cooldowns(destination_cooldowns);
        return Err(error);
    }
    if let Err(error) = store
        .flush_destination_cooldowns(destination_cooldowns.clone())
        .await
    {
        RateLimitRegistry::new().requeue_dirty_destination_cooldowns(destination_cooldowns);
        return Err(error);
    }
    Ok(())
}

fn requeue_dirty_entries(state: &Arc<Mutex<SchedulerState>>, keys: Vec<SchedulerStateKey>) {
    let mut state = state.lock().expect("upstream scheduler lock poisoned");
    state.dirty.extend(keys);
}

fn requeue_dirty_rss_cadence(state: &Arc<Mutex<SchedulerState>>, keys: Vec<SchedulerStateKey>) {
    let mut state = state.lock().expect("upstream scheduler lock poisoned");
    state.dirty_rss_cadence.extend(keys);
}

fn row_to_scheduler_entry(row: &SqlRow) -> AppResult<(SchedulerStateKey, SchedulerStateEntry)> {
    let account_quota_key = row.text("account_quota_key")?.trim().to_string();
    Ok((
        SchedulerStateKey {
            host_key: HostKey::from(row.text("host_key")?),
            destination_key: DestinationKey::from(row.text("destination_key")?),
            account_quota_key: (!account_quota_key.is_empty())
                .then(|| AccountQuotaKey::from(account_quota_key)),
            rss_request_key: row
                .opt_text("rss_request_key")?
                .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_string())),
        },
        SchedulerStateEntry {
            last_decision: row.opt_text("last_decision")?,
            last_feedback_at: row.opt_timestamp("last_feedback_at")?,
            last_successful_at: row.opt_timestamp("last_successful_at")?,
            last_attempt_at: row.opt_timestamp("last_attempt_at")?,
            api_current: row.opt_i64("api_current")?.map(|value| value.max(0) as u64),
            api_max: row.opt_i64("api_max")?.map(|value| value.max(0) as u64),
            grab_current: row
                .opt_i64("grab_current")?
                .map(|value| value.max(0) as u64),
            grab_max: row.opt_i64("grab_max")?.map(|value| value.max(0) as u64),
            quota_observed_at: row.opt_timestamp("quota_observed_at")?,
            quota_probe_after: row.opt_timestamp("quota_probe_after")?,
            quota_reset_at: row.opt_timestamp("quota_reset_at")?,
            quota_source: row.opt_text("quota_source")?,
            admitted_count: row.i64("admitted_count")?.max(0) as u64,
            deferred_count: row.i64("deferred_count")?.max(0) as u64,
            skipped_count: row.i64("skipped_count")?.max(0) as u64,
        },
    ))
}

fn candidate_state_key(candidate: &SchedulerCandidate) -> SchedulerStateKey {
    SchedulerStateKey {
        host_key: candidate.host_key.clone(),
        destination_key: candidate.destination_key.clone(),
        account_quota_key: candidate.account_quota_key.clone(),
        rss_request_key: candidate.rss_request_key.clone(),
    }
}

fn decide_candidate(
    candidate: SchedulerCandidate,
    now: DateTime<Utc>,
    state_entry: Option<&SchedulerStateEntry>,
) -> SchedulerAdmission {
    if candidate.cancel_token.is_cancelled() {
        return SchedulerAdmission::Skip {
            candidate_id: candidate.candidate_id,
            reason: SkipReason::Cancelled,
        };
    }

    if candidate
        .deadline_at
        .is_some_and(|deadline| deadline <= now)
    {
        return SchedulerAdmission::Skip {
            candidate_id: candidate.candidate_id,
            reason: SkipReason::DeadlineExpired,
        };
    }

    if candidate
        .learning_context
        .as_ref()
        .is_some_and(|context| context.suppressed)
    {
        return SchedulerAdmission::Skip {
            candidate_id: candidate.candidate_id,
            reason: SkipReason::LearningSuppressed,
        };
    }

    if let Some(retry_after) = account_quota_retry_after(&candidate, state_entry, now) {
        return SchedulerAdmission::Defer {
            candidate_id: candidate.candidate_id,
            retry_after,
            reason: DeferralReason::AccountQuotaProbePending,
        };
    }

    if RateLimitRegistry::new()
        .active_destination_cooldown(&candidate.destination_key)
        .is_some()
    {
        return SchedulerAdmission::Skip {
            candidate_id: candidate.candidate_id,
            reason: SkipReason::DestinationCooldown,
        };
    }

    if candidate.deadline_at.is_some_and(|deadline| {
        RateLimitRegistry::new()
            .preview_host_rps_wait(&candidate.host_key)
            .and_then(|wait| chrono::Duration::from_std(wait).ok())
            .is_some_and(|wait| now + wait >= deadline)
    }) {
        return SchedulerAdmission::Skip {
            candidate_id: candidate.candidate_id,
            reason: SkipReason::HostRpsDeadline,
        };
    }

    if should_defer(&candidate, now) {
        let reason = deferral_reason(&candidate);
        let retry_after = retry_after_for_deferral(&candidate);
        return SchedulerAdmission::Defer {
            candidate_id: candidate.candidate_id,
            retry_after,
            reason,
        };
    }

    let reason = admission_reason(&candidate, now);
    SchedulerAdmission::Admit {
        candidate_id: candidate.candidate_id.clone(),
        lease: SchedulerLease {
            lease_id: Uuid::new_v4().to_string(),
            candidate_id: candidate.candidate_id,
            host_key: candidate.host_key,
            destination_key: candidate.destination_key,
            account_quota_key: candidate.account_quota_key,
            rss_request_key: candidate.rss_request_key,
            operation: candidate.operation,
            intent: candidate.intent,
            issued_at: now,
        },
        reason,
    }
}

fn account_quota_retry_after(
    candidate: &SchedulerCandidate,
    state_entry: Option<&SchedulerStateEntry>,
    now: DateTime<Utc>,
) -> Option<Option<Duration>> {
    if candidate.intent == SchedulerIntent::InteractiveSearch {
        return None;
    }

    let Some(entry) = state_entry else {
        return None;
    };

    if quota_is_stale(entry, now) || quota_probe_is_due(entry, now) {
        return None;
    }

    if quota_has_capacity_for(candidate, entry) {
        return None;
    }

    Some(entry.quota_probe_after.and_then(|probe_after| {
        (probe_after > now)
            .then(|| (probe_after - now).to_std().ok())
            .flatten()
    }))
}

fn quota_has_capacity_for(candidate: &SchedulerCandidate, entry: &SchedulerStateEntry) -> bool {
    if candidate.estimated_cost.api_calls > 0.0
        && let (Some(current), Some(max)) = (entry.api_current, entry.api_max)
        && max > 0
        && (max.saturating_sub(current.min(max)) as f64) < candidate.estimated_cost.api_calls
    {
        return false;
    }

    if candidate.estimated_cost.grab_calls > 0.0
        && let (Some(current), Some(max)) = (entry.grab_current, entry.grab_max)
        && max > 0
        && (max.saturating_sub(current.min(max)) as f64) < candidate.estimated_cost.grab_calls
    {
        return false;
    }

    true
}

fn observed_quota_exhausted(entry: &SchedulerStateEntry) -> bool {
    let api_exhausted = matches!((entry.api_current, entry.api_max), (Some(current), Some(max)) if max > 0 && current >= max);
    let grab_exhausted = matches!((entry.grab_current, entry.grab_max), (Some(current), Some(max)) if max > 0 && current >= max);
    api_exhausted || grab_exhausted
}

fn quota_is_stale(entry: &SchedulerStateEntry, now: DateTime<Utc>) -> bool {
    if entry.quota_reset_at.is_some_and(|reset_at| reset_at <= now) {
        return true;
    }

    let Some(observed_at) = entry.quota_observed_at else {
        return true;
    };

    let Ok(age) = (now - observed_at).to_std() else {
        return false;
    };
    age >= QUOTA_OBSERVATION_STALE_AFTER
}

fn quota_probe_is_due(entry: &SchedulerStateEntry, now: DateTime<Utc>) -> bool {
    entry
        .quota_probe_after
        .is_some_and(|probe_after| probe_after <= now)
}

fn rss_target_interval_for_quota(api_remaining: Option<f64>, quota_exhausted: bool) -> Duration {
    if quota_exhausted {
        return EXHAUSTED_QUOTA_PROBE_AFTER;
    }

    if api_remaining.is_some_and(|remaining| remaining <= LOW_ACCOUNT_QUOTA_REMAINING_FRACTION) {
        return LOW_QUOTA_RSS_TARGET_INTERVAL;
    }

    DEFAULT_RSS_TARGET_INTERVAL
}

fn destination_cooldown_until(
    destination: &DestinationKey,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let remaining = RateLimitRegistry::new().active_destination_cooldown(destination)?;
    chrono::Duration::from_std(remaining)
        .ok()
        .map(|duration| now + duration)
}

fn should_defer(candidate: &SchedulerCandidate, now: DateTime<Utc>) -> bool {
    match candidate.intent {
        SchedulerIntent::InteractiveSearch => false,
        SchedulerIntent::BackgroundAcquisition => {
            candidate.expected_value.score < LOW_VALUE_BACKGROUND_THRESHOLD
                && !candidate
                    .learning_context
                    .as_ref()
                    .is_some_and(|context| context.historically_useful)
        }
        SchedulerIntent::BackgroundRss => candidate.freshness.as_ref().is_some_and(|freshness| {
            now < freshness.latest_safe_poll_at
                && freshness.freshness_risk < RSS_FRESHNESS_ESCALATION_THRESHOLD
        }),
        SchedulerIntent::SubtitleSearch | SchedulerIntent::SubtitleDownload => {
            candidate.expected_value.score < LOW_VALUE_SUBTITLE_THRESHOLD
        }
        SchedulerIntent::Maintenance => {
            candidate.expected_value.score < LOW_VALUE_SUBTITLE_THRESHOLD
        }
    }
}

fn admission_reason(candidate: &SchedulerCandidate, now: DateTime<Utc>) -> AdmissionReason {
    match candidate.intent {
        SchedulerIntent::InteractiveSearch => AdmissionReason::InteractiveDeadline,
        SchedulerIntent::BackgroundAcquisition => {
            if candidate.expected_value.score >= 1.0 {
                AdmissionReason::HighCapacity
            } else {
                AdmissionReason::BackgroundValue
            }
        }
        SchedulerIntent::BackgroundRss => {
            if candidate
                .freshness
                .as_ref()
                .is_some_and(|freshness| now >= freshness.latest_safe_poll_at)
            {
                AdmissionReason::RssFreshness
            } else {
                AdmissionReason::BackgroundValue
            }
        }
        SchedulerIntent::SubtitleSearch | SchedulerIntent::SubtitleDownload => {
            AdmissionReason::SubtitleAllowed
        }
        SchedulerIntent::Maintenance => AdmissionReason::MaintenanceAllowed,
    }
}

fn deferral_reason(candidate: &SchedulerCandidate) -> DeferralReason {
    match candidate.intent {
        SchedulerIntent::BackgroundRss => DeferralReason::RssCadence,
        SchedulerIntent::SubtitleSearch | SchedulerIntent::SubtitleDownload => {
            DeferralReason::SubtitleYieldedToAcquisition
        }
        SchedulerIntent::Maintenance => DeferralReason::MaintenanceLowPriority,
        _ => DeferralReason::LowCapacityBackground,
    }
}

fn retry_after_for_deferral(candidate: &SchedulerCandidate) -> Option<Duration> {
    if let Some(freshness) = candidate.freshness.as_ref()
        && let Ok(delay) = (freshness.latest_safe_poll_at - Utc::now()).to_std()
    {
        return Some(delay);
    }
    Some(Duration::from_secs(60))
}

fn candidate_score(candidate: &SchedulerCandidate, now: DateTime<Utc>) -> f64 {
    let intent = match candidate.intent {
        SchedulerIntent::InteractiveSearch => 100.0,
        SchedulerIntent::BackgroundAcquisition => 70.0,
        SchedulerIntent::BackgroundRss => 55.0,
        SchedulerIntent::SubtitleDownload => 35.0,
        SchedulerIntent::SubtitleSearch => 30.0,
        SchedulerIntent::Maintenance => 10.0,
    };
    let learned = candidate
        .learning_context
        .as_ref()
        .map(|context| {
            if context.historically_useful {
                10.0
            } else {
                0.0
            }
        })
        .unwrap_or_default();
    let freshness = candidate
        .freshness
        .as_ref()
        .map(|freshness| {
            if now >= freshness.latest_safe_poll_at {
                30.0
            } else {
                freshness.freshness_risk.clamp(0.0, 1.0) * 15.0
            }
        })
        .unwrap_or_default();

    intent + candidate.expected_value.score + learned + freshness
}

fn decision_label(decision: &SchedulerAdmission) -> &'static str {
    match decision {
        SchedulerAdmission::Admit { reason, .. } => admission_reason_label(*reason),
        SchedulerAdmission::Defer { reason, .. } => deferral_reason_label(*reason),
        SchedulerAdmission::Skip { reason, .. } => skip_reason_label(*reason),
    }
}

fn admission_reason_label(reason: AdmissionReason) -> &'static str {
    match reason {
        AdmissionReason::InteractiveDeadline => "admit:interactive_deadline",
        AdmissionReason::HighCapacity => "admit:high_capacity",
        AdmissionReason::BackgroundValue => "admit:background_value",
        AdmissionReason::RssFreshness => "admit:rss_freshness",
        AdmissionReason::SubtitleAllowed => "admit:subtitle_allowed",
        AdmissionReason::MaintenanceAllowed => "admit:maintenance_allowed",
    }
}

fn deferral_reason_label(reason: DeferralReason) -> &'static str {
    match reason {
        DeferralReason::LowCapacityBackground => "defer:low_capacity_background",
        DeferralReason::DestinationRecentlyUsed => "defer:destination_recently_used",
        DeferralReason::RssCadence => "defer:rss_cadence",
        DeferralReason::SubtitleYieldedToAcquisition => "defer:subtitle_yielded_to_acquisition",
        DeferralReason::MaintenanceLowPriority => "defer:maintenance_low_priority",
        DeferralReason::AccountQuotaProbePending => "defer:account_quota_probe_pending",
    }
}

fn skip_reason_label(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::Cancelled => "skip:cancelled",
        SkipReason::DeadlineExpired => "skip:deadline_expired",
        SkipReason::LearningSuppressed => "skip:learning_suppressed",
        SkipReason::AccountQuotaExhausted => "skip:account_quota_exhausted",
        SkipReason::DestinationCooldown => "skip:destination_cooldown",
        SkipReason::HostRpsDeadline => "skip:host_rps_deadline",
        SkipReason::HostUnavailable => "skip:host_unavailable",
    }
}

fn api_remaining_fraction(entry: &SchedulerStateEntry) -> Option<f64> {
    let max = entry.api_max?;
    if max == 0 {
        return None;
    }
    let current = entry.api_current.unwrap_or_default().min(max);
    Some((max - current) as f64 / max as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_application::{
        EstimatedCost, ExpectedValueHint, RssFreshnessContext, SchedulerCandidateId,
        SchedulerOperation, SchedulerPluginKind,
    };
    use tokio_util::sync::CancellationToken;

    fn candidate(intent: SchedulerIntent, score: f64) -> SchedulerCandidate {
        SchedulerCandidate {
            candidate_id: SchedulerCandidateId::new(),
            plugin_config_id: Some("plugin-a".to_string()),
            plugin_kind: SchedulerPluginKind::Indexer,
            operation: if intent == SchedulerIntent::BackgroundRss {
                SchedulerOperation::Rss
            } else {
                SchedulerOperation::Search
            },
            intent,
            host_key: "example.test".into(),
            destination_key: "example.test".into(),
            account_quota_key: Some("indexer-a".into()),
            rss_request_key: None,
            estimated_cost: EstimatedCost::ONE_API_CALL,
            expected_value: ExpectedValueHint { score },
            learning_context: None,
            deadline_at: None,
            freshness: None,
            cancel_token: CancellationToken::new(),
        }
    }

    fn quota_feedback(
        candidate: &SchedulerCandidate,
        observed_api_current: Option<u64>,
        observed_api_max: Option<u64>,
        observed_at: DateTime<Utc>,
    ) -> SchedulerFeedback {
        SchedulerFeedback {
            lease: None,
            host_key: candidate.host_key.clone(),
            destination_key: candidate.destination_key.clone(),
            account_quota_key: candidate.account_quota_key.clone(),
            outcome: SchedulerFeedbackOutcome::Success,
            observed_api_current,
            observed_api_max,
            observed_grab_current: None,
            observed_grab_max: None,
            retry_after: None,
            cooldown_action: RateLimitCooldownAction::None,
            rss_last_seen_release_identity: None,
            rss_last_seen_release_published_at: None,
            rss_feed_result_count: None,
            rss_seen_release_identities: Vec::new(),
            observed_at,
        }
    }

    #[tokio::test]
    async fn interactive_low_value_candidate_is_admitted() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "batch".to_string(),
                now: Utc::now(),
                candidates: vec![candidate(SchedulerIntent::InteractiveSearch, 0.0)],
            })
            .await
            .expect("admission should succeed");

        assert!(matches!(
            decision.decisions.as_slice(),
            [SchedulerAdmission::Admit {
                reason: AdmissionReason::InteractiveDeadline,
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn low_value_background_candidate_is_deferred() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "batch".to_string(),
                now: Utc::now(),
                candidates: vec![candidate(SchedulerIntent::BackgroundAcquisition, 0.0)],
            })
            .await
            .expect("admission should succeed");

        assert!(matches!(
            decision.decisions.as_slice(),
            [SchedulerAdmission::Defer {
                reason: DeferralReason::LowCapacityBackground,
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn deferred_decisions_are_visible_in_snapshot() {
        let scheduler = InMemoryUpstreamScheduler::new();
        scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "batch".to_string(),
                now: Utc::now(),
                candidates: vec![candidate(SchedulerIntent::BackgroundAcquisition, 0.0)],
            })
            .await
            .expect("admission should succeed");

        let snapshot = scheduler
            .snapshot(SchedulerSnapshotFilter::default())
            .await
            .expect("snapshot should succeed");

        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].deferred_count, 1);
        assert_eq!(
            snapshot.entries[0].last_decision.as_deref(),
            Some("defer:low_capacity_background")
        );
    }

    #[tokio::test]
    async fn rss_defer_persists_cadence_without_attempt() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let now = Utc::now();
        let mut candidate = candidate(SchedulerIntent::BackgroundRss, 0.0);
        let unique_host = HostKey::from(format!("rss-defer-{}.example.test", Uuid::new_v4()));
        candidate.host_key = unique_host.clone();
        candidate.destination_key = DestinationKey::from(unique_host.to_string());
        candidate.account_quota_key = Some(format!("rss-defer-{}", Uuid::new_v4()).into());
        candidate.freshness = Some(RssFreshnessContext {
            last_successful_poll_at: Some(now - chrono::Duration::minutes(20)),
            last_attempt_at: Some(now - chrono::Duration::minutes(10)),
            target_interval: Duration::from_secs(900),
            latest_safe_poll_at: now + chrono::Duration::minutes(30),
            estimated_feed_depth: Some(100),
            freshness_risk: 0.2,
            destination_recent_activity_at: Some(now - chrono::Duration::minutes(5)),
            account_quota_budget: Some(0.5),
        });

        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "rss-batch".to_string(),
                now,
                candidates: vec![candidate],
            })
            .await
            .expect("admission should succeed");

        assert!(matches!(
            decision.decisions.as_slice(),
            [SchedulerAdmission::Defer {
                reason: DeferralReason::RssCadence,
                ..
            }]
        ));

        let snapshot = scheduler
            .snapshot(SchedulerSnapshotFilter::default())
            .await
            .expect("snapshot should succeed");
        let entry = snapshot
            .entries
            .first()
            .expect("deferred RSS candidate should be visible");

        assert_eq!(entry.deferred_count, 1);
        assert_eq!(entry.rss_last_attempt_at, None);
        assert_eq!(entry.rss_target_interval, Some(Duration::from_secs(900)));
        assert_eq!(
            entry.rss_latest_safe_poll_at,
            Some(now + chrono::Duration::minutes(30))
        );
        assert_eq!(entry.rss_estimated_feed_depth, Some(100));
        assert_eq!(entry.rss_freshness_risk, Some(0.2));
        assert_eq!(
            entry.rss_destination_recent_activity_at,
            Some(now - chrono::Duration::minutes(5))
        );
    }

    #[tokio::test]
    async fn failed_feedback_records_attempt_but_not_success() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let candidate = candidate(SchedulerIntent::BackgroundRss, 1.0);
        let observed_at = Utc::now();
        scheduler
            .record_feedback(SchedulerFeedback {
                lease: None,
                host_key: candidate.host_key.clone(),
                destination_key: candidate.destination_key.clone(),
                account_quota_key: candidate.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::RateLimited,
                observed_api_current: None,
                observed_api_max: None,
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: Some(Duration::from_secs(30)),
                cooldown_action: RateLimitCooldownAction::RecordFallback,
                rss_last_seen_release_identity: None,
                rss_last_seen_release_published_at: None,
                rss_feed_result_count: None,
                rss_seen_release_identities: Vec::new(),
                observed_at,
            })
            .await
            .expect("feedback should persist");

        let snapshot = scheduler
            .snapshot(SchedulerSnapshotFilter::default())
            .await
            .expect("snapshot should succeed");

        assert_eq!(snapshot.entries[0].last_attempt_at, Some(observed_at));
        assert_eq!(snapshot.entries[0].last_successful_at, None);
        assert!(snapshot.entries[0].cooldown_until.is_some());
    }

    #[tokio::test]
    async fn active_destination_cooldown_skips_candidate() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let candidate = candidate(SchedulerIntent::BackgroundAcquisition, 1.0);
        scheduler
            .record_feedback(SchedulerFeedback {
                lease: None,
                host_key: candidate.host_key.clone(),
                destination_key: candidate.destination_key.clone(),
                account_quota_key: candidate.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::RateLimited,
                observed_api_current: None,
                observed_api_max: None,
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: Some(Duration::from_secs(60)),
                cooldown_action: RateLimitCooldownAction::RecordFallback,
                rss_last_seen_release_identity: None,
                rss_last_seen_release_published_at: None,
                rss_feed_result_count: None,
                rss_seen_release_identities: Vec::new(),
                observed_at: Utc::now(),
            })
            .await
            .expect("feedback should persist");

        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "batch".to_string(),
                now: Utc::now(),
                candidates: vec![candidate],
            })
            .await
            .expect("admission should succeed");

        assert!(matches!(
            decision.decisions.as_slice(),
            [SchedulerAdmission::Skip {
                reason: SkipReason::DestinationCooldown,
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn already_recorded_rate_limit_feedback_does_not_record_destination_cooldown() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let mut candidate = candidate(SchedulerIntent::BackgroundAcquisition, 1.0);
        let host = HostKey::from(format!("already-recorded-{}.example.test", Uuid::new_v4()));
        candidate.host_key = host.clone();
        candidate.destination_key = DestinationKey::from(host.to_string());
        candidate.account_quota_key = Some(AccountQuotaKey::from(format!(
            "already-recorded-{}",
            Uuid::new_v4()
        )));

        scheduler
            .record_feedback(SchedulerFeedback {
                lease: None,
                host_key: candidate.host_key.clone(),
                destination_key: candidate.destination_key.clone(),
                account_quota_key: candidate.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::RateLimited,
                observed_api_current: None,
                observed_api_max: None,
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: Some(Duration::from_secs(60)),
                cooldown_action: RateLimitCooldownAction::AlreadyRecorded,
                rss_last_seen_release_identity: None,
                rss_last_seen_release_published_at: None,
                rss_feed_result_count: None,
                rss_seen_release_identities: Vec::new(),
                observed_at: Utc::now(),
            })
            .await
            .expect("feedback should persist");

        assert!(
            RateLimitRegistry::new()
                .active_destination_cooldown(&candidate.destination_key)
                .is_none(),
            "scheduler feedback must not duplicate cooldowns already recorded by outbound HTTP"
        );
    }

    #[tokio::test]
    async fn fallback_rate_limit_feedback_records_destination_cooldown_without_retry_after() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let mut candidate = candidate(SchedulerIntent::BackgroundAcquisition, 1.0);
        let host = HostKey::from(format!("fallback-cooldown-{}.example.test", Uuid::new_v4()));
        candidate.host_key = host.clone();
        candidate.destination_key = DestinationKey::from(host.to_string());
        candidate.account_quota_key = Some(AccountQuotaKey::from(format!(
            "fallback-cooldown-{}",
            Uuid::new_v4()
        )));

        scheduler
            .record_feedback(SchedulerFeedback {
                lease: None,
                host_key: candidate.host_key.clone(),
                destination_key: candidate.destination_key.clone(),
                account_quota_key: candidate.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::RateLimited,
                observed_api_current: None,
                observed_api_max: None,
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::RecordFallback,
                rss_last_seen_release_identity: None,
                rss_last_seen_release_published_at: None,
                rss_feed_result_count: None,
                rss_seen_release_identities: Vec::new(),
                observed_at: Utc::now(),
            })
            .await
            .expect("feedback should persist");

        assert!(
            RateLimitRegistry::new()
                .active_destination_cooldown(&candidate.destination_key)
                .is_some(),
            "provider-only rate-limit feedback still needs a registry fallback cooldown"
        );
    }

    #[tokio::test]
    async fn host_rps_wait_past_deadline_skips_interactive_candidate() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let host = HostKey::from(format!("host-rps-{}.example.test", Uuid::new_v4()));
        let destination = DestinationKey::from(host.to_string());
        let registry = RateLimitRegistry::new();

        assert_eq!(registry.acquire_host_rps(&host).await, None);
        assert_eq!(registry.acquire_host_rps(&host).await, None);
        assert_eq!(registry.acquire_host_rps(&host).await, None);
        assert!(registry.preview_host_rps_wait(&host).is_some());

        let now = Utc::now();
        let mut candidate = candidate(SchedulerIntent::InteractiveSearch, 1.0);
        candidate.host_key = host;
        candidate.destination_key = destination;
        candidate.account_quota_key = Some(AccountQuotaKey::from(format!(
            "host-rps-{}",
            Uuid::new_v4()
        )));
        candidate.deadline_at = Some(now + chrono::Duration::milliseconds(10));

        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "batch".to_string(),
                now,
                candidates: vec![candidate],
            })
            .await
            .expect("admission should succeed");

        assert!(matches!(
            decision.decisions.as_slice(),
            [SchedulerAdmission::Skip {
                reason: SkipReason::HostRpsDeadline,
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn fresh_account_quota_exhaustion_defers_background_candidate() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let candidate = candidate(SchedulerIntent::BackgroundAcquisition, 1.0);
        scheduler
            .record_feedback(SchedulerFeedback {
                lease: None,
                host_key: candidate.host_key.clone(),
                destination_key: candidate.destination_key.clone(),
                account_quota_key: candidate.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::Success,
                observed_api_current: Some(10),
                observed_api_max: Some(10),
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::None,
                rss_last_seen_release_identity: None,
                rss_last_seen_release_published_at: None,
                rss_feed_result_count: None,
                rss_seen_release_identities: Vec::new(),
                observed_at: Utc::now(),
            })
            .await
            .expect("feedback should persist");

        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "batch".to_string(),
                now: Utc::now(),
                candidates: vec![candidate],
            })
            .await
            .expect("admission should succeed");

        assert!(matches!(
            decision.decisions.as_slice(),
            [SchedulerAdmission::Defer {
                reason: DeferralReason::AccountQuotaProbePending,
                retry_after: Some(_),
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn fresh_account_quota_exhaustion_does_not_block_interactive_probe() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let mut candidate = candidate(SchedulerIntent::InteractiveSearch, 1.0);
        scheduler
            .record_feedback(SchedulerFeedback {
                lease: None,
                host_key: candidate.host_key.clone(),
                destination_key: candidate.destination_key.clone(),
                account_quota_key: candidate.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::Success,
                observed_api_current: Some(10),
                observed_api_max: Some(10),
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::None,
                rss_last_seen_release_identity: None,
                rss_last_seen_release_published_at: None,
                rss_feed_result_count: None,
                rss_seen_release_identities: Vec::new(),
                observed_at: Utc::now(),
            })
            .await
            .expect("feedback should persist");

        candidate.intent = SchedulerIntent::InteractiveSearch;
        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "batch".to_string(),
                now: Utc::now(),
                candidates: vec![candidate],
            })
            .await
            .expect("admission should succeed");

        assert!(matches!(
            decision.decisions.as_slice(),
            [SchedulerAdmission::Admit {
                reason: AdmissionReason::InteractiveDeadline,
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn stale_account_quota_exhaustion_allows_background_probe() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let candidate = candidate(SchedulerIntent::BackgroundAcquisition, 1.0);
        let now = Utc::now();
        scheduler
            .record_feedback(SchedulerFeedback {
                lease: None,
                host_key: candidate.host_key.clone(),
                destination_key: candidate.destination_key.clone(),
                account_quota_key: candidate.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::Success,
                observed_api_current: Some(10),
                observed_api_max: Some(10),
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::None,
                rss_last_seen_release_identity: None,
                rss_last_seen_release_published_at: None,
                rss_feed_result_count: None,
                rss_seen_release_identities: Vec::new(),
                observed_at: now - chrono::Duration::hours(25),
            })
            .await
            .expect("feedback should persist");

        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "batch".to_string(),
                now,
                candidates: vec![candidate],
            })
            .await
            .expect("admission should succeed");

        assert!(matches!(
            decision.decisions.as_slice(),
            [SchedulerAdmission::Admit {
                reason: AdmissionReason::HighCapacity,
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn newer_quota_observation_replaces_exhausted_quota() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let candidate = candidate(SchedulerIntent::BackgroundAcquisition, 1.0);
        let now = Utc::now();

        scheduler
            .record_feedback(quota_feedback(
                &candidate,
                Some(10),
                Some(10),
                now - chrono::Duration::minutes(1),
            ))
            .await
            .expect("exhausted feedback should persist");
        scheduler
            .record_feedback(quota_feedback(&candidate, Some(1), Some(10), now))
            .await
            .expect("fresh feedback should persist");

        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "batch".to_string(),
                now,
                candidates: vec![candidate],
            })
            .await
            .expect("admission should succeed");

        assert!(matches!(
            decision.decisions.as_slice(),
            [SchedulerAdmission::Admit {
                reason: AdmissionReason::HighCapacity,
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn older_exhausted_quota_observation_does_not_overwrite_newer_capacity() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let candidate = candidate(SchedulerIntent::BackgroundAcquisition, 1.0);
        let now = Utc::now();

        scheduler
            .record_feedback(quota_feedback(&candidate, Some(1), Some(10), now))
            .await
            .expect("fresh feedback should persist");
        scheduler
            .record_feedback(quota_feedback(
                &candidate,
                Some(10),
                Some(10),
                now - chrono::Duration::minutes(1),
            ))
            .await
            .expect("older feedback should not replace newer state");

        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "batch".to_string(),
                now,
                candidates: vec![candidate],
            })
            .await
            .expect("admission should succeed");

        assert!(matches!(
            decision.decisions.as_slice(),
            [SchedulerAdmission::Admit {
                reason: AdmissionReason::HighCapacity,
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn background_batch_conserves_low_capacity_after_sufficient_coverage() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let mut low_capacity = candidate(SchedulerIntent::BackgroundAcquisition, 0.0);
        low_capacity.host_key = "low-cap.example".into();
        low_capacity.destination_key = "low-cap.example".into();
        low_capacity.account_quota_key = Some("low-cap".into());

        scheduler
            .record_feedback(SchedulerFeedback {
                lease: None,
                host_key: low_capacity.host_key.clone(),
                destination_key: low_capacity.destination_key.clone(),
                account_quota_key: low_capacity.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::Success,
                observed_api_current: Some(90),
                observed_api_max: Some(100),
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::None,
                rss_last_seen_release_identity: None,
                rss_last_seen_release_published_at: None,
                rss_feed_result_count: None,
                rss_seen_release_identities: Vec::new(),
                observed_at: Utc::now(),
            })
            .await
            .expect("feedback should persist");

        let mut high_a = candidate(SchedulerIntent::BackgroundAcquisition, 2.0);
        high_a.account_quota_key = Some("high-a".into());
        let mut high_b = candidate(SchedulerIntent::BackgroundAcquisition, 2.0);
        high_b.host_key = "high-b.example".into();
        high_b.destination_key = "high-b.example".into();
        high_b.account_quota_key = Some("high-b".into());
        let mut high_c = candidate(SchedulerIntent::BackgroundAcquisition, 2.0);
        high_c.host_key = "high-c.example".into();
        high_c.destination_key = "high-c.example".into();
        high_c.account_quota_key = Some("high-c".into());

        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "batch".to_string(),
                now: Utc::now(),
                candidates: vec![low_capacity, high_a, high_b, high_c],
            })
            .await
            .expect("admission should succeed");

        assert_eq!(
            decision
                .decisions
                .iter()
                .filter(|decision| matches!(decision, SchedulerAdmission::Admit { .. }))
                .count(),
            3
        );
        assert!(decision.decisions.iter().any(|decision| matches!(
            decision,
            SchedulerAdmission::Defer {
                reason: DeferralReason::LowCapacityBackground,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn rss_escalates_at_latest_safe_poll() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let now = Utc::now();
        let mut rss = candidate(SchedulerIntent::BackgroundRss, 0.0);
        rss.freshness = Some(RssFreshnessContext {
            last_successful_poll_at: None,
            last_attempt_at: None,
            target_interval: Duration::from_secs(900),
            latest_safe_poll_at: now,
            estimated_feed_depth: None,
            freshness_risk: 1.0,
            destination_recent_activity_at: None,
            account_quota_budget: None,
        });

        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "batch".to_string(),
                now,
                candidates: vec![rss],
            })
            .await
            .expect("admission should succeed");

        assert!(matches!(
            decision.decisions.as_slice(),
            [SchedulerAdmission::Admit {
                reason: AdmissionReason::RssFreshness,
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn rss_feedback_records_cadence_and_feed_gap() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let now = Utc::now();
        let rss = candidate(SchedulerIntent::BackgroundRss, 1.0);
        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "batch".to_string(),
                now,
                candidates: vec![rss.clone()],
            })
            .await
            .expect("admission should succeed");
        let lease = match decision.decisions.into_iter().next() {
            Some(SchedulerAdmission::Admit { lease, .. }) => lease,
            other => panic!("expected RSS admit, got {other:?}"),
        };

        scheduler
            .record_feedback(SchedulerFeedback {
                lease: Some(lease.clone()),
                host_key: rss.host_key.clone(),
                destination_key: rss.destination_key.clone(),
                account_quota_key: rss.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::Success,
                observed_api_current: None,
                observed_api_max: None,
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::None,
                rss_last_seen_release_identity: Some("old-guid".to_string()),
                rss_last_seen_release_published_at: Some(now),
                rss_feed_result_count: Some(10),
                rss_seen_release_identities: vec!["old-guid".to_string(), "older-guid".to_string()],
                observed_at: now,
            })
            .await
            .expect("feedback should record");

        let later = now + chrono::Duration::minutes(20);
        scheduler
            .record_feedback(SchedulerFeedback {
                lease: Some(lease),
                host_key: rss.host_key.clone(),
                destination_key: rss.destination_key.clone(),
                account_quota_key: rss.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::Success,
                observed_api_current: None,
                observed_api_max: None,
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::None,
                rss_last_seen_release_identity: Some("new-guid".to_string()),
                rss_last_seen_release_published_at: Some(later),
                rss_feed_result_count: Some(5),
                rss_seen_release_identities: vec!["new-guid".to_string(), "newer-guid".to_string()],
                observed_at: later,
            })
            .await
            .expect("feedback should record");

        let snapshot = scheduler
            .snapshot(SchedulerSnapshotFilter::default())
            .await
            .expect("snapshot should succeed");
        let entry = snapshot.entries.first().expect("snapshot entry");
        assert_eq!(
            entry.rss_last_seen_release_identity.as_deref(),
            Some("new-guid")
        );
        assert_eq!(entry.rss_estimated_feed_depth, Some(5));
        assert_eq!(entry.rss_last_feed_gap_start_at, Some(now));
        assert_eq!(entry.rss_last_feed_gap_end_at, Some(later));
    }

    #[tokio::test]
    async fn rss_request_keys_keep_feed_gap_state_separate() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let now = Utc::now();
        let host = HostKey::from(format!("rss-request-key-{}.example.test", Uuid::new_v4()));
        let destination = DestinationKey::from(host.to_string());
        let account = AccountQuotaKey::from(format!("rss-request-key-{}", Uuid::new_v4()));

        let mut anime = candidate(SchedulerIntent::BackgroundRss, 1.0);
        anime.host_key = host.clone();
        anime.destination_key = destination.clone();
        anime.account_quota_key = Some(account.clone());
        anime.rss_request_key = Some("rss:5070".to_string());

        let mut movies = candidate(SchedulerIntent::BackgroundRss, 1.0);
        movies.host_key = host;
        movies.destination_key = destination;
        movies.account_quota_key = Some(account);
        movies.rss_request_key = Some("rss:2000".to_string());

        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "rss-request-keys".to_string(),
                now,
                candidates: vec![anime.clone(), movies.clone()],
            })
            .await
            .expect("admission should succeed");
        let mut anime_lease = None;
        let mut movies_lease = None;
        for decision in decision.decisions {
            if let SchedulerAdmission::Admit { lease, .. } = decision {
                match lease.rss_request_key.as_deref() {
                    Some("rss:5070") => anime_lease = Some(lease),
                    Some("rss:2000") => movies_lease = Some(lease),
                    other => panic!("unexpected RSS request key {other:?}"),
                }
            }
        }
        let anime_lease = anime_lease.expect("anime RSS candidate should be admitted");
        let movies_lease = movies_lease.expect("movie RSS candidate should be admitted");

        scheduler
            .record_feedback(SchedulerFeedback {
                lease: Some(anime_lease.clone()),
                host_key: anime.host_key.clone(),
                destination_key: anime.destination_key.clone(),
                account_quota_key: anime.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::Success,
                observed_api_current: None,
                observed_api_max: None,
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::None,
                rss_last_seen_release_identity: Some("anime-old".to_string()),
                rss_last_seen_release_published_at: Some(now),
                rss_feed_result_count: Some(2),
                rss_seen_release_identities: vec!["anime-old".to_string()],
                observed_at: now,
            })
            .await
            .expect("anime feedback should record");
        scheduler
            .record_feedback(SchedulerFeedback {
                lease: Some(movies_lease),
                host_key: movies.host_key.clone(),
                destination_key: movies.destination_key.clone(),
                account_quota_key: movies.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::Success,
                observed_api_current: None,
                observed_api_max: None,
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::None,
                rss_last_seen_release_identity: Some("movie-old".to_string()),
                rss_last_seen_release_published_at: Some(now),
                rss_feed_result_count: Some(2),
                rss_seen_release_identities: vec!["movie-old".to_string()],
                observed_at: now,
            })
            .await
            .expect("movie feedback should record");

        let later = now + chrono::Duration::minutes(20);
        scheduler
            .record_feedback(SchedulerFeedback {
                lease: Some(anime_lease),
                host_key: anime.host_key.clone(),
                destination_key: anime.destination_key.clone(),
                account_quota_key: anime.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::Success,
                observed_api_current: None,
                observed_api_max: None,
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::None,
                rss_last_seen_release_identity: Some("anime-new".to_string()),
                rss_last_seen_release_published_at: Some(later),
                rss_feed_result_count: Some(1),
                rss_seen_release_identities: vec!["anime-new".to_string()],
                observed_at: later,
            })
            .await
            .expect("second anime feedback should record");

        let snapshot = scheduler
            .snapshot(SchedulerSnapshotFilter::default())
            .await
            .expect("snapshot should succeed");
        let anime_entry = snapshot
            .entries
            .iter()
            .find(|entry| entry.rss_request_key.as_deref() == Some("rss:5070"))
            .expect("anime RSS entry should exist");
        let movie_entry = snapshot
            .entries
            .iter()
            .find(|entry| entry.rss_request_key.as_deref() == Some("rss:2000"))
            .expect("movie RSS entry should exist");

        assert_eq!(anime_entry.rss_last_feed_gap_start_at, Some(now));
        assert_eq!(anime_entry.rss_last_feed_gap_end_at, Some(later));
        assert_eq!(movie_entry.rss_last_feed_gap_start_at, None);
        assert_eq!(
            movie_entry.rss_last_seen_release_identity.as_deref(),
            Some("movie-old")
        );
    }

    #[tokio::test]
    async fn rss_feedback_widens_cadence_for_low_or_exhausted_quota() {
        let scheduler = InMemoryUpstreamScheduler::new();
        let now = Utc::now();
        let rss = candidate(SchedulerIntent::BackgroundRss, 1.0);
        let decision = scheduler
            .admit_batch(SchedulerBatchRequest {
                batch_id: "batch".to_string(),
                now,
                candidates: vec![rss.clone()],
            })
            .await
            .expect("admission should succeed");
        let lease = match decision.decisions.into_iter().next() {
            Some(SchedulerAdmission::Admit { lease, .. }) => lease,
            other => panic!("expected RSS admit, got {other:?}"),
        };

        scheduler
            .record_feedback(SchedulerFeedback {
                lease: Some(lease.clone()),
                host_key: rss.host_key.clone(),
                destination_key: rss.destination_key.clone(),
                account_quota_key: rss.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::Success,
                observed_api_current: Some(90),
                observed_api_max: Some(100),
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::None,
                rss_last_seen_release_identity: Some("low-quota-guid".to_string()),
                rss_last_seen_release_published_at: Some(now),
                rss_feed_result_count: Some(1),
                rss_seen_release_identities: vec!["low-quota-guid".to_string()],
                observed_at: now,
            })
            .await
            .expect("feedback should record");
        let snapshot = scheduler
            .snapshot(SchedulerSnapshotFilter::default())
            .await
            .expect("snapshot should succeed");
        assert_eq!(
            snapshot
                .entries
                .first()
                .and_then(|entry| entry.rss_target_interval),
            Some(LOW_QUOTA_RSS_TARGET_INTERVAL)
        );

        let exhausted_at = now + chrono::Duration::minutes(1);
        scheduler
            .record_feedback(SchedulerFeedback {
                lease: Some(lease.clone()),
                host_key: rss.host_key.clone(),
                destination_key: rss.destination_key.clone(),
                account_quota_key: rss.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::Success,
                observed_api_current: Some(100),
                observed_api_max: Some(100),
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::None,
                rss_last_seen_release_identity: Some("exhausted-quota-guid".to_string()),
                rss_last_seen_release_published_at: Some(exhausted_at),
                rss_feed_result_count: Some(1),
                rss_seen_release_identities: vec!["exhausted-quota-guid".to_string()],
                observed_at: exhausted_at,
            })
            .await
            .expect("feedback should record");
        let snapshot = scheduler
            .snapshot(SchedulerSnapshotFilter::default())
            .await
            .expect("snapshot should succeed");
        assert_eq!(
            snapshot
                .entries
                .first()
                .and_then(|entry| entry.rss_target_interval),
            Some(EXHAUSTED_QUOTA_PROBE_AFTER)
        );

        let healthy_at = now + chrono::Duration::minutes(2);
        scheduler
            .record_feedback(SchedulerFeedback {
                lease: Some(lease),
                host_key: rss.host_key.clone(),
                destination_key: rss.destination_key.clone(),
                account_quota_key: rss.account_quota_key.clone(),
                outcome: SchedulerFeedbackOutcome::Success,
                observed_api_current: Some(10),
                observed_api_max: Some(100),
                observed_grab_current: None,
                observed_grab_max: None,
                retry_after: None,
                cooldown_action: RateLimitCooldownAction::None,
                rss_last_seen_release_identity: Some("healthy-quota-guid".to_string()),
                rss_last_seen_release_published_at: Some(healthy_at),
                rss_feed_result_count: Some(1),
                rss_seen_release_identities: vec!["healthy-quota-guid".to_string()],
                observed_at: healthy_at,
            })
            .await
            .expect("feedback should record");
        let snapshot = scheduler
            .snapshot(SchedulerSnapshotFilter::default())
            .await
            .expect("snapshot should succeed");
        assert_eq!(
            snapshot
                .entries
                .first()
                .and_then(|entry| entry.rss_target_interval),
            Some(DEFAULT_RSS_TARGET_INTERVAL)
        );
    }
}
