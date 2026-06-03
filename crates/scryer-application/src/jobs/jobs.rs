use super::*;
use crate::domain_events::new_job_run_domain_event;
use crate::event_views::replay_active_job_runs;
use chrono::Utc;
use scryer_domain::{
    DomainEventFilter, DomainEventPayload, DomainEventType, JobNextRunUpdatedEventData,
    JobRunCompletedEventData, JobRunFailedEventData, JobRunStartedEventData,
};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use tracing::{info, warn};

const BACKGROUND_LIBRARY_REFRESH_INTERVAL_SECONDS: i64 = 3600;
const BACKGROUND_LIBRARY_REFRESH_STAGGER_SECONDS: i64 = 15 * 60;

fn is_background_library_refresh_job(job_key: JobKey) -> bool {
    matches!(
        job_key,
        JobKey::BackgroundLibraryRefreshMovies
            | JobKey::BackgroundLibraryRefreshSeries
            | JobKey::BackgroundLibraryRefreshAnime
    )
}

fn library_job_operation_type(job_key: JobKey, library_id: &str) -> String {
    format!("{}:{library_id}", job_key.as_str())
}

fn job_run_library_id(run: &JobRunRecord) -> Option<&str> {
    run.operation_type
        .strip_prefix(run.job_key.as_str())
        .and_then(|value| value.strip_prefix(':'))
        .filter(|value| !value.trim().is_empty())
}

fn background_library_refresh_enabled() -> bool {
    std::env::var("SCRYER_BACKGROUND_LIBRARY_REFRESH")
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "false" | "0" | "no" | "off"
            )
        })
        .unwrap_or(true)
}

#[derive(Clone, Debug, serde::Serialize)]
struct MetadataRefreshSummary {
    refreshed_titles: u32,
}

#[derive(Clone, Debug, serde::Serialize)]
struct HealthChecksSummary {
    total: usize,
    errors: usize,
    warnings: usize,
    checks: Vec<HealthCheckSummaryItem>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct HealthCheckSummaryItem {
    source: String,
    status: String,
    message: String,
}

#[derive(Clone, Debug, serde::Serialize)]
struct CountSummary {
    count: u32,
}

#[derive(Clone, Debug, serde::Serialize)]
struct LibraryScanRunSummary {
    scanned: usize,
    matched: usize,
    imported: usize,
    skipped: usize,
    unmatched: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
struct RssSyncRunSummary {
    releases_fetched: usize,
    releases_matched: usize,
    releases_grabbed: usize,
    releases_held: usize,
}

#[derive(Clone, Debug, serde::Serialize)]
struct HousekeepingRunSummary {
    orphaned_media_files: u32,
    stale_release_decisions: u32,
    stale_release_attempts: u32,
    expired_event_outboxes: u32,
    stale_history_events: u32,
    stale_history_records: u32,
    staged_nzb_artifacts_pruned: u32,
    recycled_purged: u32,
}

#[derive(Clone, Debug)]
struct JobExecutionOutcome {
    summary_text: Option<String>,
    summary_json: Option<String>,
    library_scan_progress: Option<LibraryScanSession>,
    status_override: Option<JobRunStatus>,
}

impl JobExecutionOutcome {
    fn new(summary_text: Option<String>, summary_json: Option<String>) -> Self {
        Self {
            summary_text,
            summary_json,
            library_scan_progress: None,
            status_override: None,
        }
    }

    fn warning(summary_text: Option<String>, summary_json: Option<String>) -> Self {
        Self {
            summary_text,
            summary_json,
            library_scan_progress: None,
            status_override: Some(JobRunStatus::Warning),
        }
    }

    fn from_library_scan(summary: &LibraryScanSummary) -> Self {
        Self::new(
            Some(summary_text_from_library_scan(summary)),
            serde_json::to_string(&LibraryScanRunSummary {
                scanned: summary.scanned,
                matched: summary.matched,
                imported: summary.imported,
                skipped: summary.skipped,
                unmatched: summary.unmatched,
            })
            .ok(),
        )
    }
}

impl AppUseCase {
    async fn load_active_job_run_projection(&self) -> AppResult<Vec<JobRun>> {
        let mut events = Vec::new();
        let mut after_sequence = 0i64;

        loop {
            let batch = self
                .services
                .events
                .domain_events
                .list(&DomainEventFilter {
                    after_sequence: Some(after_sequence),
                    event_types: Some(vec![
                        DomainEventType::JobRunStarted,
                        DomainEventType::JobRunCompleted,
                        DomainEventType::JobRunFailed,
                        DomainEventType::LibraryScanStarted,
                        DomainEventType::LibraryScanProgressed,
                        DomainEventType::LibraryScanCompleted,
                        DomainEventType::LibraryScanCanceled,
                        DomainEventType::LibraryScanFailed,
                    ]),
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
            events.extend(batch);
            if count < 500 {
                break;
            }
        }

        let mut runs = replay_active_job_runs(&events)
            .into_values()
            .collect::<Vec<_>>();
        runs.sort_by_key(|run| run.started_at);
        Ok(runs)
    }

    pub async fn list_jobs(&self, actor: &User) -> AppResult<Vec<JobDefinition>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let next_runs = self.runtime.jobs.job_run_tracker.all_next_runs().await;
        Ok(crate::jobs::all_job_definitions(&next_runs))
    }

    pub async fn active_job_runs(&self, actor: &User) -> AppResult<Vec<JobRun>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let runs = self.runtime.jobs.job_run_tracker.list_active().await;
        if runs.is_empty() {
            self.load_active_job_run_projection().await
        } else {
            Ok(runs)
        }
    }

    pub async fn list_job_runs(
        &self,
        actor: &User,
        job_key: JobKey,
        limit: usize,
    ) -> AppResult<Vec<JobRun>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let active_runs = {
            let runs = self.runtime.jobs.job_run_tracker.list_active().await;
            if runs.is_empty() {
                self.load_active_job_run_projection().await?
            } else {
                runs
            }
        };
        let active_runs_by_id = active_runs
            .into_iter()
            .map(|run| (run.id.clone(), run))
            .collect::<HashMap<_, _>>();

        let records = self
            .services
            .events
            .job_runs
            .list_job_runs(Some(job_key), limit.max(1))
            .await?;

        Ok(records
            .into_iter()
            .map(|record| {
                active_runs_by_id
                    .get(&record.id)
                    .cloned()
                    .unwrap_or_else(|| JobRun::from_record(&record, None))
            })
            .collect())
    }

    pub async fn list_recent_job_runs(&self, actor: &User, limit: usize) -> AppResult<Vec<JobRun>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let active_runs = {
            let runs = self.runtime.jobs.job_run_tracker.list_active().await;
            if runs.is_empty() {
                self.load_active_job_run_projection().await?
            } else {
                runs
            }
        };
        let active_runs_by_id = active_runs
            .into_iter()
            .map(|run| (run.id.clone(), run))
            .collect::<HashMap<_, _>>();

        let records = self
            .services
            .events
            .job_runs
            .list_job_runs(None, limit.max(1))
            .await?;

        Ok(records
            .into_iter()
            .map(|record| {
                active_runs_by_id
                    .get(&record.id)
                    .cloned()
                    .unwrap_or_else(|| JobRun::from_record(&record, None))
            })
            .collect())
    }

    pub async fn subscribe_job_run_events(
        &self,
        actor: &User,
    ) -> AppResult<broadcast::Receiver<JobRun>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let (tx, rx) = broadcast::channel(128);
        let app = self.clone();
        tokio::spawn(async move {
            let mut receiver = app.runtime.jobs.job_run_tracker.subscribe();
            let mut initial_runs = app.runtime.jobs.job_run_tracker.list_active().await;
            if initial_runs.is_empty() {
                initial_runs = match app.load_active_job_run_projection().await {
                    Ok(runs) => runs,
                    Err(error) => {
                        tracing::warn!("job run subscription initial load failed: {error}");
                        return;
                    }
                };
            }
            for run in initial_runs {
                if tx.send(run).is_err() {
                    return;
                }
            }

            loop {
                match receiver.recv().await {
                    Ok(run) => {
                        if tx.send(run).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!("job run subscription lagged, skipped {n} tracker updates");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Ok(rx)
    }

    pub async fn trigger_job(&self, actor: &User, job_key: JobKey) -> AppResult<JobRun> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        if !job_key.manual_trigger_allowed() {
            return Err(AppError::Validation(format!(
                "{} can only run on its configured schedule",
                job_key.display_name()
            )));
        }
        self.ensure_job_can_start(job_key).await?;

        let run = self
            .create_job_run_record(
                job_key,
                JobTriggerSource::Manual,
                Some(actor.id.clone()),
                None,
            )
            .await?;
        let run_payload = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(run_payload.clone())
            .await;
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                Some(actor.id.clone()),
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_string(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_string(),
                }),
            ))
            .await;

        let app = self.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            if let Err(error) = app.run_job_run(run, Some(actor)).await {
                warn!(job_key = job_key.as_str(), error = %error, "manual job trigger failed");
            }
        });

        Ok(run_payload)
    }

    pub async fn run_scheduled_job_now(
        &self,
        job_key: JobKey,
        trigger_source: JobTriggerSource,
    ) -> AppResult<()> {
        if is_background_library_refresh_job(job_key) {
            return self
                .run_scheduled_background_library_refresh_jobs_now(job_key, trigger_source)
                .await;
        }

        self.ensure_job_can_start(job_key).await?;
        let run = self
            .create_job_run_record(job_key, trigger_source, None, None)
            .await?;
        let run_payload = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(run_payload)
            .await;
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                None,
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_string(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_string(),
                }),
            ))
            .await;
        self.run_job_run(run, None).await
    }

    async fn run_scheduled_background_library_refresh_jobs_now(
        &self,
        job_key: JobKey,
        trigger_source: JobTriggerSource,
    ) -> AppResult<()> {
        let facet = job_key_library_facet(job_key).expect("background refresh facet");
        let libraries = self.services.catalog.libraries.list(Some(facet)).await?;
        if libraries.is_empty() {
            return Err(AppError::Validation(format!(
                "{} has no libraries to refresh",
                job_key.display_name()
            )));
        }

        let actor = self.find_or_create_default_user().await?;
        let mut first_error = None;
        for library in libraries {
            if let Err(error) = self.ensure_job_can_start(job_key).await {
                if first_error.is_none() {
                    first_error = Some(error.to_string());
                }
                continue;
            }

            let run = self
                .create_job_run_record(
                    job_key,
                    trigger_source,
                    None,
                    Some(library_job_operation_type(job_key, &library.id)),
                )
                .await?;
            let run_payload = JobRun::from_record(&run, None);
            self.runtime
                .jobs
                .job_run_tracker
                .upsert_active_run(run_payload)
                .await;
            let _ = self
                .append_domain_event(new_job_run_domain_event(
                    None,
                    run.id.clone(),
                    DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                        run_id: run.id.clone(),
                        job_key: run.job_key.as_str().to_string(),
                        operation_type: run.operation_type.clone(),
                        trigger_source: run.trigger_source.as_str().to_string(),
                    }),
                ))
                .await;

            if let Err(error) = self.run_job_run(run, Some(actor.clone())).await
                && first_error.is_none()
            {
                first_error = Some(error.to_string());
            }
        }

        if let Some(error) = first_error {
            Err(AppError::Validation(error))
        } else {
            Ok(())
        }
    }

    pub async fn set_job_next_run_at(&self, job_key: JobKey, next_run_at: chrono::DateTime<Utc>) {
        self.runtime
            .jobs
            .job_run_tracker
            .set_next_run_at(job_key, next_run_at)
            .await;
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                None,
                job_key.as_str().to_string(),
                DomainEventPayload::JobNextRunUpdated(JobNextRunUpdatedEventData {
                    job_key: job_key.as_str().to_string(),
                    next_run_at: Some(next_run_at.to_rfc3339()),
                }),
            ))
            .await;
    }

    pub async fn clear_job_next_run_at(&self, job_key: JobKey) {
        self.runtime
            .jobs
            .job_run_tracker
            .clear_next_run_at(job_key)
            .await;
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                None,
                job_key.as_str().to_string(),
                DomainEventPayload::JobNextRunUpdated(JobNextRunUpdatedEventData {
                    job_key: job_key.as_str().to_string(),
                    next_run_at: None,
                }),
            ))
            .await;
    }

    async fn ensure_job_can_start(&self, job_key: JobKey) -> AppResult<()> {
        if self
            .runtime
            .jobs
            .job_run_tracker
            .has_active_job(job_key)
            .await
        {
            return Err(AppError::Validation(format!(
                "{} is already running",
                job_key.display_name()
            )));
        }

        if let Some(facet) = job_key_library_facet(job_key) {
            let active_scans = self
                .runtime
                .library
                .library_scan_tracker
                .list_active()
                .await;
            if active_scans
                .into_iter()
                .any(|session| session.facet == facet)
            {
                return Err(AppError::Validation(format!(
                    "{} library scan is already running",
                    facet.as_str()
                )));
            }
        }

        Ok(())
    }

    async fn create_job_run_record(
        &self,
        job_key: JobKey,
        trigger_source: JobTriggerSource,
        actor_user_id: Option<String>,
        operation_type: Option<String>,
    ) -> AppResult<JobRunRecord> {
        let now = Utc::now();
        let initial_status = if job_key.uses_library_scan_progress() {
            JobRunStatus::Discovering
        } else {
            JobRunStatus::Running
        };

        self.services
            .events
            .job_runs
            .create_job_run(&JobRunRecord {
                id: Id::new().0,
                job_key,
                operation_type: operation_type.unwrap_or_else(|| job_key.as_str().to_string()),
                status: initial_status,
                trigger_source,
                actor_user_id,
                progress_json: Some(json!({ "status": initial_status.as_str() }).to_string()),
                summary_json: None,
                summary_text: None,
                error_text: None,
                started_at: now,
                completed_at: None,
                created_at: now,
                updated_at: now,
            })
            .await
    }

    async fn run_job_run(&self, run: JobRunRecord, actor: Option<User>) -> AppResult<()> {
        match self.execute_job_body(&run, actor).await {
            Ok(outcome) => {
                self.finish_job_run(
                    run,
                    outcome.summary_text,
                    outcome.summary_json,
                    outcome.library_scan_progress,
                    outcome.status_override,
                )
                .await
            }
            Err(error) => {
                self.fail_job_run(run, error.to_string()).await?;
                Err(error)
            }
        }
    }

    async fn execute_job_body(
        &self,
        run: &JobRunRecord,
        actor: Option<User>,
    ) -> AppResult<JobExecutionOutcome> {
        let job_key = run.job_key;
        let run_id = run.id.as_str();
        match job_key {
            JobKey::LibraryScanMovies | JobKey::LibraryScanSeries | JobKey::LibraryScanAnime => {
                let actor = match actor {
                    Some(actor) => actor,
                    None => self.find_or_create_default_user().await?,
                };
                let facet = job_key_library_facet(job_key).expect("library scan facet");
                let summary = self
                    .scan_library_with_tracking(
                        &actor,
                        facet,
                        Some(run_id.to_string()),
                        LibraryScanMode::Full,
                    )
                    .await?;
                Ok(JobExecutionOutcome::from_library_scan(&summary))
            }
            JobKey::BackgroundLibraryRefreshMovies
            | JobKey::BackgroundLibraryRefreshSeries
            | JobKey::BackgroundLibraryRefreshAnime => {
                if !background_library_refresh_enabled() {
                    return Err(AppError::Validation(
                        "background library refresh is temporarily disabled".into(),
                    ));
                }
                let actor = match actor {
                    Some(actor) => actor,
                    None => self.find_or_create_default_user().await?,
                };
                let summary = if let Some(library_id) = job_run_library_id(run) {
                    self.background_library_refresh_by_id_with_tracking(&actor, library_id, run_id)
                        .await?
                } else {
                    let facet = job_key_library_facet(job_key).expect("background refresh facet");
                    self.background_library_refresh_with_tracking(&actor, facet, run_id)
                        .await?
                };
                Ok(JobExecutionOutcome::from_library_scan(&summary))
            }
            JobKey::ProwlarrSync => {
                let actor = match actor {
                    Some(actor) => actor,
                    None => self.find_or_create_default_user().await?,
                };
                let (synced_count, failures) = self.sync_enabled_prowlarr_indexers(&actor).await?;
                if failures.is_empty() {
                    Ok(JobExecutionOutcome::new(
                        Some(format!("Synced {synced_count} enabled Prowlarr parent(s)")),
                        serde_json::to_string(&CountSummary {
                            count: synced_count,
                        })
                        .ok(),
                    ))
                } else {
                    Ok(JobExecutionOutcome::warning(
                        Some(format!(
                            "Synced {synced_count} enabled Prowlarr parent(s); {} failed",
                            failures.len()
                        )),
                        serde_json::to_string(&json!({
                            "syncedCount": synced_count,
                            "failedCount": failures.len(),
                            "failures": failures,
                        }))
                        .ok(),
                    ))
                }
            }
            JobKey::RssSync => {
                let report = self.run_scheduled_rss_sync().await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!(
                        "Fetched {}, matched {}, grabbed {}",
                        report.releases_fetched, report.releases_matched, report.releases_grabbed
                    )),
                    serde_json::to_string(&RssSyncRunSummary {
                        releases_fetched: report.releases_fetched,
                        releases_matched: report.releases_matched,
                        releases_grabbed: report.releases_grabbed,
                        releases_held: report.releases_held,
                    })
                    .ok(),
                ))
            }
            JobKey::SubtitleSearch => Ok(JobExecutionOutcome::new(
                Some(self.run_subtitle_search_job().await?),
                None,
            )),
            JobKey::MetadataRefresh => {
                let refreshed_titles = self.run_metadata_refresh_job().await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!("Refreshed metadata for {refreshed_titles} titles")),
                    serde_json::to_string(&MetadataRefreshSummary { refreshed_titles }).ok(),
                ))
            }
            JobKey::PluginRegistryRefresh => {
                self.refresh_plugin_catalog_internal().await?;
                Ok(JobExecutionOutcome::new(
                    Some("Plugin catalog refreshed".to_string()),
                    None,
                ))
            }
            JobKey::Housekeeping => {
                let report = self.run_scheduled_housekeeping().await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!(
                        "Removed {} orphaned media files and {} stale release decisions",
                        report.orphaned_media_files, report.stale_release_decisions
                    )),
                    serde_json::to_string(&HousekeepingRunSummary {
                        orphaned_media_files: report.orphaned_media_files,
                        stale_release_decisions: report.stale_release_decisions,
                        stale_release_attempts: report.stale_release_attempts,
                        expired_event_outboxes: report.expired_event_outboxes,
                        stale_history_events: report.stale_history_events,
                        stale_history_records: report.stale_history_records,
                        staged_nzb_artifacts_pruned: report.staged_nzb_artifacts_pruned,
                        recycled_purged: report.recycled_purged,
                    })
                    .ok(),
                ))
            }
            JobKey::HealthChecks => {
                let results = self.run_health_checks().await;
                *self.runtime.health.results.write().await = results.clone();
                let errors = results
                    .iter()
                    .filter(|result| matches!(result.status, HealthCheckStatus::Error))
                    .count();
                let warnings = results
                    .iter()
                    .filter(|result| matches!(result.status, HealthCheckStatus::Warning))
                    .count();
                Ok(JobExecutionOutcome::new(
                    Some(format!(
                        "Completed {} health checks ({} errors, {} warnings)",
                        results.len(),
                        errors,
                        warnings
                    )),
                    serde_json::to_string(&HealthChecksSummary {
                        total: results.len(),
                        errors,
                        warnings,
                        checks: results
                            .iter()
                            .map(|result| HealthCheckSummaryItem {
                                source: result.source.clone(),
                                status: result.status.as_str().to_string(),
                                message: result.message.clone(),
                            })
                            .collect(),
                    })
                    .ok(),
                ))
            }
            JobKey::AutoBackup => match self.run_auto_backup_job().await? {
                crate::security::backup::AutoBackupRunOutcome::Created { info, pruned_count } => {
                    let summary_text = Some(format!(
                        "Created {} ({}) and pruned {} older automatic backup{}",
                        info.filename,
                        if info.encrypted {
                            "encrypted"
                        } else {
                            "plaintext"
                        },
                        pruned_count,
                        if pruned_count == 1 { "" } else { "s" },
                    ));
                    let summary_json = serde_json::json!({
                        "filename": info.filename,
                        "encrypted": info.encrypted,
                        "prunedCount": pruned_count,
                        "trigger": info.trigger.as_str(),
                    })
                    .to_string();
                    Ok(JobExecutionOutcome::new(summary_text, Some(summary_json)))
                }
                crate::security::backup::AutoBackupRunOutcome::Skipped { reason } => {
                    Ok(JobExecutionOutcome::warning(Some(reason), None))
                }
            },
            JobKey::WantedSync => {
                self.sync_wanted_state().await?;
                Ok(JobExecutionOutcome::new(
                    Some("Wanted state synchronized".to_string()),
                    None,
                ))
            }
            JobKey::PendingReleaseProcessing => {
                let count = self.process_expired_pending_releases().await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!("Processed {count} pending releases")),
                    serde_json::to_string(&CountSummary { count }).ok(),
                ))
            }
            JobKey::StagedNzbPrune => {
                let count = self
                    .services
                    .workflow
                    .staged_nzb_store
                    .prune_staged_nzbs_older_than(Utc::now() - chrono::Duration::hours(1))
                    .await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!("Pruned {count} staged NZB artifacts")),
                    serde_json::to_string(&CountSummary { count }).ok(),
                ))
            }
            JobKey::TitleImageCacheRefresh => {
                let summary = self.run_title_image_cache_refresh().await?;
                Ok(JobExecutionOutcome::new(
                    Some(format!(
                        "Refreshed artwork URLs for {} title(s) and {} episode(s); image cache reset",
                        summary.title_urls_updated, summary.episode_urls_updated
                    )),
                    serde_json::to_string(&summary).ok(),
                ))
            }
        }
    }

    async fn finish_job_run(
        &self,
        mut run: JobRunRecord,
        summary_text: Option<String>,
        summary_json: Option<String>,
        library_scan_progress: Option<LibraryScanSession>,
        status_override: Option<JobRunStatus>,
    ) -> AppResult<()> {
        let completed_at = Utc::now();
        run.status = status_override.unwrap_or_else(|| {
            match library_scan_progress
                .as_ref()
                .map(|session| &session.status)
            {
                Some(LibraryScanStatus::Warning) => JobRunStatus::Warning,
                Some(LibraryScanStatus::Canceled) => JobRunStatus::Warning,
                Some(LibraryScanStatus::Failed) => JobRunStatus::Failed,
                _ => JobRunStatus::Completed,
            }
        });
        run.progress_json = Some(json!({ "status": run.status.as_str() }).to_string());
        run.summary_text = summary_text;
        run.summary_json = summary_json;
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        let updated = self.services.events.job_runs.update_job_run(&run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, library_scan_progress))
            .await;
        let payload = if matches!(run.status, JobRunStatus::Failed) {
            DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                run_id: run.id.clone(),
                job_key: run.job_key.as_str().to_string(),
                error_text: run.error_text.clone(),
            })
        } else {
            DomainEventPayload::JobRunCompleted(JobRunCompletedEventData {
                run_id: run.id.clone(),
                job_key: run.job_key.as_str().to_string(),
                summary_text: run.summary_text.clone(),
            })
        };
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                updated.actor_user_id.clone(),
                updated.id.clone(),
                payload,
            ))
            .await;
        Ok(())
    }

    async fn fail_job_run(&self, mut run: JobRunRecord, error_text: String) -> AppResult<()> {
        let completed_at = Utc::now();
        run.status = JobRunStatus::Failed;
        run.progress_json = Some(json!({ "status": run.status.as_str() }).to_string());
        run.error_text = Some(error_text.clone());
        run.summary_text = Some(format!("Failed: {error_text}"));
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        let updated = self.services.events.job_runs.update_job_run(&run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        let _ = self
            .append_domain_event(new_job_run_domain_event(
                updated.actor_user_id.clone(),
                updated.id.clone(),
                DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                    run_id: updated.id.clone(),
                    job_key: updated.job_key.as_str().to_string(),
                    error_text: updated.error_text.clone(),
                }),
            ))
            .await;
        Ok(())
    }
}

pub async fn start_background_library_refresh_loop(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    if !background_library_refresh_enabled() {
        info!(
            "background library refresh loop is disabled (SCRYER_BACKGROUND_LIBRARY_REFRESH=false)"
        );
        return;
    }

    let startup_seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default();

    for job_key in [
        JobKey::BackgroundLibraryRefreshMovies,
        JobKey::BackgroundLibraryRefreshSeries,
        JobKey::BackgroundLibraryRefreshAnime,
    ] {
        let app = app.clone();
        let token = token.child_token();
        let initial_delay_seconds =
            background_library_refresh_initial_delay_seconds(startup_seed, job_key)
                .expect("background refresh job");
        tokio::spawn(async move {
            run_background_library_refresh_worker(app, token, job_key, initial_delay_seconds).await;
        });
    }

    token.cancelled().await;
}

async fn run_background_library_refresh_worker(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
    job_key: JobKey,
    initial_delay_seconds: i64,
) {
    let initial_delay = initial_delay_seconds.max(1) as u64;
    let interval_seconds = job_key
        .interval_seconds()
        .unwrap_or(BACKGROUND_LIBRARY_REFRESH_INTERVAL_SECONDS)
        .max(1) as u64;
    let initial_next_run_at = Utc::now() + chrono::Duration::seconds(initial_delay as i64);
    app.set_job_next_run_at(job_key, initial_next_run_at).await;

    tokio::select! {
        _ = token.cancelled() => return,
        _ = tokio::time::sleep(std::time::Duration::from_secs(initial_delay)) => {}
    }

    if let Err(error) = app
        .run_scheduled_job_now(job_key, JobTriggerSource::ScheduledStartup)
        .await
    {
        warn!(job_key = job_key.as_str(), error = %error, "startup background job failed");
    }

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_seconds));
    interval.tick().await;
    app.set_job_next_run_at(
        job_key,
        Utc::now() + chrono::Duration::seconds(interval_seconds as i64),
    )
    .await;

    loop {
        tokio::select! {
            _ = token.cancelled() => return,
            _ = interval.tick() => {
                app.set_job_next_run_at(
                    job_key,
                    Utc::now() + chrono::Duration::seconds(interval_seconds as i64),
                ).await;
                if let Err(error) = app
                    .run_scheduled_job_now(job_key, JobTriggerSource::ScheduledInterval)
                    .await
                {
                    warn!(job_key = job_key.as_str(), error = %error, "scheduled background job failed");
                }
            }
        }
    }
}

fn background_library_refresh_initial_delay_seconds(
    startup_seed: u64,
    job_key: JobKey,
) -> Option<i64> {
    let order = match job_key {
        JobKey::BackgroundLibraryRefreshMovies => 0,
        JobKey::BackgroundLibraryRefreshSeries => 1,
        JobKey::BackgroundLibraryRefreshAnime => 2,
        _ => return None,
    };

    let randomized_base =
        (startup_seed % BACKGROUND_LIBRARY_REFRESH_INTERVAL_SECONDS as u64) as i64;
    Some(randomized_base + order * BACKGROUND_LIBRARY_REFRESH_STAGGER_SECONDS)
}

fn job_key_library_facet(job_key: JobKey) -> Option<MediaFacet> {
    match job_key {
        JobKey::LibraryScanMovies | JobKey::BackgroundLibraryRefreshMovies => {
            Some(MediaFacet::Movie)
        }
        JobKey::LibraryScanSeries | JobKey::BackgroundLibraryRefreshSeries => {
            Some(MediaFacet::Series)
        }
        JobKey::LibraryScanAnime | JobKey::BackgroundLibraryRefreshAnime => Some(MediaFacet::Anime),
        _ => None,
    }
}

fn summary_text_from_library_scan(summary: &LibraryScanSummary) -> String {
    format!(
        "Scanned {}, imported {}, skipped {}, unmatched {}",
        summary.scanned, summary.imported, summary.skipped, summary.unmatched
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_library_refresh_initial_delays_are_staggered_by_facet() {
        let startup_seed = 1_234_u64;
        let movie = background_library_refresh_initial_delay_seconds(
            startup_seed,
            JobKey::BackgroundLibraryRefreshMovies,
        )
        .expect("movie delay");
        let series = background_library_refresh_initial_delay_seconds(
            startup_seed,
            JobKey::BackgroundLibraryRefreshSeries,
        )
        .expect("series delay");
        let anime = background_library_refresh_initial_delay_seconds(
            startup_seed,
            JobKey::BackgroundLibraryRefreshAnime,
        )
        .expect("anime delay");

        assert_eq!(movie, 1_234);
        assert_eq!(series - movie, BACKGROUND_LIBRARY_REFRESH_STAGGER_SECONDS);
        assert_eq!(anime - series, BACKGROUND_LIBRARY_REFRESH_STAGGER_SECONDS);
    }
}
