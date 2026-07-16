//! Derived wanted/upgrades views (RFC 119 §6/§7). The Missing and Upgrades tabs
//! read the SAME derived target set the convergence cursor rotates over
//! (`derive_missing_targets` / `compute_cutoff_unmet_items`), joined to the
//! activity-driven state row when one exists and enriched with the per-scope
//! convergence progress the UI shows instead of a search cadence. Convergence is
//! derived for a whole page in ONE coverage round-trip (#12): resolve each scope's
//! `(fingerprint, routed indexers)` once per title, fetch all coverage rows for the
//! page's scope keys together, then compute covered/routed counts in memory and
//! fold in the scheduler availability snapshot for the `Deferred` state.

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use scryer_domain::{
    DomainEventPayload, Id, JobRunCompletedEventData, JobRunFailedEventData,
    JobRunStartedEventData, MediaFacet,
};

use super::*;
use crate::acquisition::convergence::convergence_scope_key;
use crate::acquisition::targets::AcquisitionTarget;
use crate::contracts::{QueueDownloadOutcome, SubmissionConflictPolicy, SubmissionScope};

/// Convergence state of a scope for the UI (RFC 119 §6). Mirrors the GraphQL
/// `ConvergenceStateValue`; the interface maps this 1:1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WantedConvergenceState {
    /// No routed indexer searched yet under the current fingerprint.
    Queued,
    /// Some but not all routed indexers covered — sweep in progress.
    Searching,
    /// Every routed indexer covered — watching RSS.
    Converged,
    /// Not converged, and every still-uncovered indexer is currently unavailable.
    Deferred,
}

/// Per-scope convergence progress carried on a wanted view (RFC 119 §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WantedViewConvergence {
    pub state: WantedConvergenceState,
    pub indexers_covered: i32,
    pub indexers_routed: i32,
}

/// One derived wanted/upgrades row (RFC 119 §6/§7): the target coordinates, the
/// joined activity-state row (when one exists), title/library enrichment, the
/// recency lane, and the batched convergence progress. `id`-identity is decided by
/// the interface mapper (state-row id, else scope key).
#[derive(Clone, Debug)]
pub struct WantedScopeView {
    pub scope_key: String,
    pub title_id: String,
    pub library_id: String,
    pub facet: MediaFacet,
    /// "movie" | "episode" | "series_movie".
    pub media_type: String,
    pub episode_id: Option<String>,
    pub collection_id: Option<String>,
    pub series_movie_link_id: Option<String>,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub title_name: Option<String>,
    pub title_slug: Option<String>,
    pub library_name: Option<String>,
    pub library_slug: Option<String>,
    /// Recency lane (`true` = hot). Upgrades are always cold.
    pub is_hot: bool,
    /// The activity-driven acquisition-state row, when one exists for this scope.
    pub state: Option<AcquisitionScopeState>,
    pub convergence: WantedViewConvergence,
}

/// Resolved `(fingerprint, facet, routed indexers)` for one title — identical
/// across all of a title's scopes (same profile, routing and match identity), so
/// it is resolved once per title and reused for every scope of that title (#12).
#[derive(Clone)]
struct TitleConvergenceContext {
    fingerprint: String,
    routed_indexer_ids: Vec<String>,
}

impl AppUseCase {
    /// One page of the derived Missing/Upgrades view (RFC 119 §6/§7). Mirrors the
    /// cutoff-unmet authorization: results are limited to the actor's authorized
    /// libraries. `MISSING` derives from the same fileless-scope query the cursor
    /// uses; `CUTOFF_UPGRADE` reuses the cutoff-unmet compute. Both join the state
    /// row (excluding paused/grabbed-active scopes), enrich title/library names,
    /// sort deterministically, then slice — the convergence progress for the sliced
    /// page is derived in one batched coverage round-trip.
    #[expect(
        clippy::too_many_arguments,
        reason = "the derived wanted view is parameterized by kind, facet, library scope, search, and paging"
    )]
    pub async fn list_wanted_scope_views(
        &self,
        actor: &User,
        kind: WantedKind,
        facet: Option<MediaFacet>,
        library_ids: Vec<String>,
        title_search: Option<String>,
        limit: i64,
        offset: i64,
    ) -> AppResult<(Vec<WantedScopeView>, i64)> {
        let authorized = self
            .list_libraries_for_permission(
                actor,
                facet.clone(),
                scryer_domain::LibraryPermission::View,
            )
            .await?;
        let mut authorized_ids: HashSet<String> = authorized
            .iter()
            .map(|library| library.id.clone())
            .collect();
        let requested: HashSet<String> = library_ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect();
        if !requested.is_empty() {
            authorized_ids.retain(|id| requested.contains(id));
        }
        if authorized_ids.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let library_name_by_id: HashMap<String, String> = authorized
            .iter()
            .map(|library| (library.id.clone(), library.name.clone()))
            .collect();
        let library_slug_by_id: HashMap<String, String> = authorized
            .iter()
            .map(|library| (library.id.clone(), library.slug.clone()))
            .collect();

        // Every non-`wanted` state row (paused or an in-flight grab) whose scope is
        // still derivable — excluded from the active view (§D-B: the view lists
        // actionable targets, not scopes already paused or grabbed).
        let excluded_scope_keys = self.non_wanted_state_scope_keys().await?;

        let facet_str = facet.as_ref().map(|facet| facet.as_str().to_string());
        let title_needle = title_search
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase);

        // Raw target coordinates for the requested kind, already availability-gated.
        let mut rows: Vec<WantedScopeView> = match kind {
            WantedKind::Missing => {
                let now = Utc::now();
                self.derive_missing_targets(&now)
                    .await?
                    .into_iter()
                    .filter(|target| authorized_ids.contains(&target.library_id))
                    .filter(|target| {
                        facet_str
                            .as_deref()
                            .is_none_or(|facet| target.facet.as_str() == facet)
                    })
                    .filter(|target| !excluded_scope_keys.contains(&target.scope_key))
                    .map(missing_target_to_view)
                    .collect()
            }
            WantedKind::CutoffUpgrade => {
                let library_filter = authorized_ids.iter().cloned().collect::<Vec<_>>();
                self.compute_cutoff_unmet_items(facet.clone(), Some(library_filter))
                    .await?
                    .into_iter()
                    .filter_map(cutoff_item_to_view)
                    .filter(|view| !excluded_scope_keys.contains(&view.scope_key))
                    .collect()
            }
        };

        // Enrich title name/slug (page-independent identity is needed for the title
        // search filter and the deterministic sort, so it is loaded before slicing;
        // one lookup per unique title).
        self.enrich_view_titles(&mut rows).await;
        for view in &mut rows {
            view.library_name = library_name_by_id.get(&view.library_id).cloned();
            view.library_slug = library_slug_by_id.get(&view.library_id).cloned();
        }

        if let Some(needle) = title_needle.as_deref() {
            rows.retain(|view| {
                view.title_name
                    .as_deref()
                    .is_some_and(|name| name.to_ascii_lowercase().contains(needle))
            });
        }

        sort_wanted_views(&mut rows);

        let total = rows.len() as i64;
        let offset = offset.max(0) as usize;
        let limit = limit.max(0) as usize;
        let mut page: Vec<WantedScopeView> = rows.into_iter().skip(offset).take(limit).collect();

        // Join the state row + derive convergence for the sliced page only.
        self.attach_state_rows(&mut page).await?;
        self.attach_page_convergence(&mut page).await?;

        Ok((page, total))
    }

    /// Scope keys whose state row is paused or grabbed — excluded from the active
    /// derived view. One list query, keyed by the same scope identity the cursor
    /// uses.
    async fn non_wanted_state_scope_keys(&self) -> AppResult<HashSet<String>> {
        let mut excluded = HashSet::new();
        for status in [
            AcquisitionScopeStatus::Paused,
            AcquisitionScopeStatus::Grabbed,
        ] {
            let items = self
                .services
                .workflow
                .acquisition_scope_states
                .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                    statuses: vec![status.as_str().to_string()],
                    limit: i64::MAX,
                    ..AcquisitionScopeStatesQuery::default()
                })
                .await?;
            for item in items {
                let scope = SubmissionScope::from_persisted(
                    &item.title_id,
                    item.episode_id.clone(),
                    item.collection_id.clone(),
                    item.series_movie_link_id.clone(),
                    None,
                );
                if let Some(scope_key) = convergence_scope_key(&scope, &item.title_id) {
                    excluded.insert(scope_key);
                }
            }
        }
        Ok(excluded)
    }

    /// Fill in `title_name`/`title_slug` for the derived rows — one `get_by_id` per
    /// unique title, cached across the row's scopes.
    async fn enrich_view_titles(&self, rows: &mut [WantedScopeView]) {
        let unique_title_ids: HashSet<String> =
            rows.iter().map(|view| view.title_id.clone()).collect();
        let mut names: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
        for title_id in unique_title_ids {
            if let Ok(Some(title)) = self.services.catalog.titles.get_by_id(&title_id).await {
                names.insert(title_id, (Some(title.name), title.slug));
            }
        }
        for view in rows.iter_mut() {
            if view.title_name.is_some() {
                continue;
            }
            if let Some((name, slug)) = names.get(&view.title_id) {
                view.title_name = name.clone();
                view.title_slug = slug.clone();
            }
        }
    }

    /// Attach the activity-driven state row for each page scope (page-sized, so a
    /// per-scope lookup is bounded).
    async fn attach_state_rows(&self, page: &mut [WantedScopeView]) -> AppResult<()> {
        for view in page.iter_mut() {
            view.state = self
                .find_wanted_state_for_scope(
                    &view.title_id,
                    view.episode_id.as_deref(),
                    view.collection_id.as_deref(),
                    view.series_movie_link_id.as_deref(),
                )
                .await?;
        }
        Ok(())
    }

    /// Derive per-scope convergence progress for a page in ONE coverage round-trip
    /// (RFC 119 §6 #12) and attach it to each view.
    async fn attach_page_convergence(&self, page: &mut [WantedScopeView]) -> AppResult<()> {
        if page.is_empty() {
            return Ok(());
        }
        let scopes: Vec<(String, String)> = page
            .iter()
            .map(|view| (view.title_id.clone(), view.scope_key.clone()))
            .collect();
        let by_scope = self.page_convergence_by_scope_key(&scopes).await;
        for view in page.iter_mut() {
            if let Some(convergence) = by_scope.get(&view.scope_key) {
                view.convergence = *convergence;
            }
        }
        Ok(())
    }

    /// Batched per-scope convergence progress for a page (RFC 119 §6 #12), keyed by
    /// scope key. Resolves `(fingerprint, routed indexers)` once per title, fetches
    /// all coverage rows for the page's scope keys in one round-trip, computes
    /// covered/routed counts in memory, and folds in the scheduler availability
    /// snapshot to distinguish `Deferred` from `Queued`. Shared by the Missing /
    /// Upgrades views and the cutoff-unmet page so both show identical convergence.
    pub async fn page_convergence_by_scope_key(
        &self,
        scopes: &[(String, String)],
    ) -> HashMap<String, WantedViewConvergence> {
        let mut result = HashMap::new();
        if scopes.is_empty() {
            return result;
        }

        // One (fingerprint, routed) resolution per unique title — identical across a
        // title's scopes.
        let mut title_context: HashMap<String, Option<TitleConvergenceContext>> = HashMap::new();
        for (title_id, _) in scopes {
            if title_context.contains_key(title_id) {
                continue;
            }
            let context = self.resolve_title_convergence_context(title_id).await;
            title_context.insert(title_id.clone(), context);
        }

        // One coverage fetch for the whole page.
        let scope_keys: Vec<String> = scopes.iter().map(|(_, key)| key.clone()).collect();
        let coverage_rows = self
            .services
            .integrations
            .scope_indexer_coverage
            .list_coverage_for_scope_keys(&scope_keys)
            .await
            .unwrap_or_default();
        let mut coverage_by_scope: HashMap<String, Vec<crate::ScopeCoverageRow>> = HashMap::new();
        for row in coverage_rows {
            coverage_by_scope
                .entry(row.scope_key.clone())
                .or_default()
                .push(row);
        }

        let availability = self.scheduler_availability().await;
        let host_keys = self.indexer_scheduler_host_keys().await;

        for (title_id, scope_key) in scopes {
            let Some(Some(context)) = title_context.get(title_id) else {
                // No routing/profile resolvable — nothing to converge; present as
                // converged (0/0) so the UI does not show a perpetual sweep.
                result.insert(
                    scope_key.clone(),
                    WantedViewConvergence {
                        state: WantedConvergenceState::Converged,
                        indexers_covered: 0,
                        indexers_routed: 0,
                    },
                );
                continue;
            };
            let routed = &context.routed_indexer_ids;
            let covered: HashSet<&str> = coverage_by_scope
                .get(scope_key)
                .map(|rows| {
                    rows.iter()
                        .filter(|row| row.fingerprint == context.fingerprint)
                        .map(|row| row.indexer_id.as_str())
                        .collect()
                })
                .unwrap_or_default();
            let covered_count = routed
                .iter()
                .filter(|id| covered.contains(id.as_str()))
                .count();
            let routed_count = routed.len();
            let uncovered: Vec<&String> = routed
                .iter()
                .filter(|id| !covered.contains(id.as_str()))
                .collect();

            let state = if uncovered.is_empty() {
                WantedConvergenceState::Converged
            } else if uncovered.iter().all(|id| {
                !availability.indexer_available(host_keys.get(id.as_str()).map(String::as_str), id)
            }) {
                WantedConvergenceState::Deferred
            } else if covered_count == 0 {
                WantedConvergenceState::Queued
            } else {
                WantedConvergenceState::Searching
            };

            result.insert(
                scope_key.clone(),
                WantedViewConvergence {
                    state,
                    indexers_covered: covered_count as i32,
                    indexers_routed: routed_count as i32,
                },
            );
        }

        result
    }

    /// Resolve `(fingerprint, routed indexers)` for a title via its title-level
    /// search subject — the values every scope of the title shares. `None` when the
    /// title is gone or nothing is routed.
    async fn resolve_title_convergence_context(
        &self,
        title_id: &str,
    ) -> Option<TitleConvergenceContext> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await
            .ok()??;
        let subject = self
            .resolve_release_search_subject_for_title(&title)
            .await
            .ok()?;
        let convergence = self.resolve_scope_convergence(&title, &subject).await?;
        Some(TitleConvergenceContext {
            fingerprint: convergence.fingerprint,
            routed_indexer_ids: convergence.routed_indexer_ids,
        })
    }
}

/// A missing target's coordinates as a view row (state/enrichment/convergence
/// filled in later).
fn missing_target_to_view(target: AcquisitionTarget) -> WantedScopeView {
    WantedScopeView {
        scope_key: target.scope_key,
        title_id: target.title_id,
        library_id: target.library_id,
        facet: target.facet,
        media_type: target.media_type,
        episode_id: target.episode_id,
        collection_id: target.collection_id,
        series_movie_link_id: target.series_movie_link_id,
        season_number: target.season_number,
        episode_number: target.episode_number,
        title_name: None,
        title_slug: None,
        library_name: None,
        library_slug: None,
        is_hot: target.is_hot,
        state: None,
        convergence: pending_convergence(),
    }
}

/// A cutoff-unmet item as a view row. Upgrades are always cold. `None` when the
/// item's scope has no derivable convergence key.
fn cutoff_item_to_view(item: CutoffUnmetItem) -> Option<WantedScopeView> {
    let scope =
        SubmissionScope::from_persisted(&item.title_id, item.episode_id.clone(), None, None, None);
    let scope_key = convergence_scope_key(&scope, &item.title_id)?;
    let media_type = if item.episode_id.is_some() {
        "episode"
    } else {
        "movie"
    };
    Some(WantedScopeView {
        scope_key,
        title_id: item.title_id,
        library_id: item.library_id,
        facet: item.title_facet,
        media_type: media_type.to_string(),
        episode_id: item.episode_id,
        collection_id: None,
        series_movie_link_id: None,
        season_number: item.season_number,
        episode_number: item.episode_number,
        title_name: Some(item.title_name),
        title_slug: item.title_slug,
        library_name: item.library_name,
        library_slug: item.library_slug,
        is_hot: false,
        state: None,
        convergence: pending_convergence(),
    })
}

/// Placeholder convergence used before the batched per-page derivation fills it in.
fn pending_convergence() -> WantedViewConvergence {
    WantedViewConvergence {
        state: WantedConvergenceState::Queued,
        indexers_covered: 0,
        indexers_routed: 0,
    }
}

/// Digits of `value`, or `i64::MAX` when absent — matches the cutoff-unmet sort so
/// Missing and Upgrades order identically.
fn parse_sort_number(value: Option<&str>) -> i64 {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| {
            let digits: String = value.chars().filter(char::is_ascii_digit).collect();
            (!digits.is_empty())
                .then(|| digits.parse::<i64>().ok())
                .flatten()
        })
        .unwrap_or(i64::MAX)
}

// ── Interactive acquisition-search job (RFC 119 §7.3 / Phase 2) ─────────────

/// One scope to search in an interactive acquisition-search job.
#[derive(Clone, Debug)]
struct AcquisitionSearchScope {
    title_id: String,
    scope: SubmissionScope,
    /// Human label for the progress `currentTitle` field.
    label: String,
}

/// Request for the interactive acquisition-search job (RFC 119 §7.3). A bare
/// request searches every derived target of `wanted_kind`; the narrowing fields
/// filter that set, and `wanted_item_id` (a state-row id or a scope key) selects a
/// single scope.
#[derive(Clone, Debug, Default)]
pub struct AcquisitionSearchRequest {
    pub wanted_kind: WantedKind,
    pub facet: Option<MediaFacet>,
    pub library_ids: Vec<String>,
    pub title_id: Option<String>,
    pub season_number: Option<i32>,
    pub wanted_item_id: Option<String>,
}

/// `Missing` is the default target set for the interactive search request (RFC 119
/// §7.3) — matching the `wantedItems` query default. Defined here because that's
/// the only consumer of a defaulted `WantedKind`.
impl Default for WantedKind {
    fn default() -> Self {
        Self::Missing
    }
}

/// Progress snapshot persisted in the job run's `progress_json` (RFC 119 §7.3),
/// read back by the `acquisitionSearchJob` query and pushed via `jobRunEvents`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquisitionSearchProgress {
    /// One of the `AcquisitionSearchJobStateValue` snake_case names.
    pub state: String,
    pub total: usize,
    pub processed: usize,
    pub grabbed_count: usize,
    pub failed_count: usize,
    pub current_title: Option<String>,
}

/// App-side view of the interactive acquisition-search job for the GraphQL query
/// (RFC 119 §7.3). Built from the persisted run record + its progress json.
#[derive(Clone, Debug)]
pub struct AcquisitionSearchJobView {
    pub id: String,
    /// Snake_case `AcquisitionSearchJobStateValue` name.
    pub state: String,
    pub total: i32,
    pub processed: i32,
    pub grabbed_count: i32,
    pub failed_count: i32,
    pub current_title: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

/// Map a terminal/running job-run status onto the acquisition-search job state
/// vocabulary (RFC 119 §7.3): a cancellation lands as `Warning`, which the UI
/// shows as `Cancelled`.
fn acquisition_search_state_for_status(status: JobRunStatus, cancelled: bool) -> &'static str {
    if cancelled {
        return "cancelled";
    }
    match status {
        JobRunStatus::Completed => "completed",
        JobRunStatus::Failed => "failed",
        JobRunStatus::Warning => "cancelled",
        _ => "running",
    }
}

impl AppUseCase {
    /// Start the interactive acquisition-search job (RFC 119 §7.3 / Phase 2):
    /// single-flight guarded, permission-checked (ManageTitles for a title-scoped
    /// request, ManageCatalogSettings for a facet/library-wide one — mirroring
    /// `scanLibrary`), then runs the per-scope best-release search+grab off a
    /// spawned task under a cancellation token. Returns the started run for the
    /// payload; progress is polled via `acquisition_search_job` and pushed via
    /// `jobRunEvents`.
    pub async fn start_acquisition_search_job(
        &self,
        actor: &User,
        request: AcquisitionSearchRequest,
    ) -> AppResult<JobRun> {
        let search_guard = self
            .runtime
            .jobs
            .acquisition_search_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| {
                AppError::Validation("an acquisition search job is already running".into())
            })?;
        if self
            .runtime
            .jobs
            .job_run_tracker
            .has_active_job(JobKey::AcquisitionSearch)
            .await
        {
            return Err(AppError::Validation(
                "an acquisition search job is already running".into(),
            ));
        }

        self.authorize_acquisition_search(actor, &request).await?;
        let scopes = self
            .resolve_acquisition_search_scopes(actor, &request)
            .await?;

        let now = chrono::Utc::now();
        let mut run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::AcquisitionSearch,
            operation_type: format!(
                "acquisition_search:{}:{}",
                request.wanted_kind.as_str(),
                scopes.len()
            ),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: serde_json::to_string(&AcquisitionSearchProgress {
                state: "running".to_string(),
                total: scopes.len(),
                processed: 0,
                grabbed_count: 0,
                failed_count: 0,
                current_title: None,
            })
            .ok(),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        run = self.services.events.job_runs.create_job_run(&run).await?;
        let run_payload = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(run_payload.clone())
            .await;

        let cancellation = tokio_util::sync::CancellationToken::new();
        self.runtime
            .acquisition
            .acquisition_search_cancellation_tokens
            .lock()
            .await
            .insert(run.id.clone(), cancellation.clone());

        let actor_event = DomainEventActor::from(actor);
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor_event.clone(),
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
            app.run_acquisition_search_job(
                run,
                actor,
                actor_event,
                scopes,
                cancellation,
                search_guard,
            )
            .await;
        });

        Ok(run_payload)
    }

    /// Permission split (RFC 119 §7.3, mirroring `scanLibrary`): a title-scoped
    /// request (an explicit `title_id`, or a `wanted_item_id` resolving to one
    /// title) requires `ManageTitles` on that title's library; a facet- or
    /// library-wide request requires `ManageCatalogSettings`.
    async fn authorize_acquisition_search(
        &self,
        actor: &User,
        request: &AcquisitionSearchRequest,
    ) -> AppResult<()> {
        if let Some(title_id) = self.acquisition_search_scoped_title(request).await? {
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(&title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
            return self
                .require_library_permission(
                    actor,
                    &title.library_id,
                    scryer_domain::LibraryPermission::ManageTitles,
                )
                .await;
        }
        self.require_app_permission(actor, AppPermission::ManageCatalogSettings)
            .await
    }

    /// The single title a request is scoped to, if any — the explicit `title_id` or
    /// the title behind a `wanted_item_id`. `None` for a facet/library-wide request.
    async fn acquisition_search_scoped_title(
        &self,
        request: &AcquisitionSearchRequest,
    ) -> AppResult<Option<String>> {
        if let Some(title_id) = request
            .title_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(Some(title_id.to_string()));
        }
        if let Some(identifier) = request
            .wanted_item_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(self
                .resolve_scope_identifier(identifier)
                .await?
                .map(|(title_id, _)| title_id));
        }
        Ok(None)
    }

    /// The set of scopes an acquisition-search request targets. `wanted_item_id`
    /// yields exactly one scope; otherwise the derived target set of the requested
    /// kind is filtered by facet/library/title/season.
    async fn resolve_acquisition_search_scopes(
        &self,
        actor: &User,
        request: &AcquisitionSearchRequest,
    ) -> AppResult<Vec<AcquisitionSearchScope>> {
        if let Some(identifier) = request
            .wanted_item_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let (title_id, scope) = self
                .resolve_scope_identifier(identifier)
                .await?
                .ok_or_else(|| {
                    AppError::NotFound(format!("no acquisition scope for '{identifier}'"))
                })?;
            let label = self.acquisition_scope_label(&title_id, &scope).await;
            return Ok(vec![AcquisitionSearchScope {
                title_id,
                scope,
                label,
            }]);
        }

        // Derive the full target set (unpaged) via the same view derivation, then
        // map each row to a searchable scope.
        let (views, _total) = self
            .list_wanted_scope_views(
                actor,
                request.wanted_kind,
                request.facet.clone(),
                request.library_ids.clone(),
                None,
                i64::MAX,
                0,
            )
            .await?;
        let season_filter = request.season_number.map(|value| value.to_string());
        let scopes = views
            .into_iter()
            .filter(|view| {
                request
                    .title_id
                    .as_deref()
                    .is_none_or(|title_id| view.title_id == title_id)
            })
            .filter(|view| {
                season_filter
                    .as_deref()
                    .is_none_or(|season| view.season_number.as_deref() == Some(season))
            })
            .filter_map(|view| {
                let scope = submission_scope_for_view(&view)?;
                Some(AcquisitionSearchScope {
                    label: view
                        .title_name
                        .clone()
                        .unwrap_or_else(|| view.title_id.clone()),
                    title_id: view.title_id,
                    scope,
                })
            })
            .collect();
        Ok(scopes)
    }

    /// Resolve a wanted identifier — a state-row id, else a convergence scope key —
    /// into `(title_id, SubmissionScope)`. Scope-key prefixes are parsed directly;
    /// an `episode:` key loads the episode to recover its title.
    pub(crate) async fn resolve_scope_identifier(
        &self,
        identifier: &str,
    ) -> AppResult<Option<(String, SubmissionScope)>> {
        let identifier = identifier.trim();
        if identifier.is_empty() {
            return Ok(None);
        }

        // State-row id first.
        if let Some(item) = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(identifier)
            .await?
        {
            let scope = SubmissionScope::from_persisted(
                &item.title_id,
                item.episode_id.clone(),
                item.collection_id.clone(),
                item.series_movie_link_id.clone(),
                None,
            );
            return Ok(Some((item.title_id, scope)));
        }

        // Otherwise a convergence scope key.
        if let Some(episode_id) = identifier.strip_prefix("episode:") {
            let Some(episode) = self
                .services
                .catalog
                .shows
                .get_episode_by_id(episode_id)
                .await?
            else {
                return Ok(None);
            };
            return Ok(Some((
                episode.title_id,
                SubmissionScope::Episode {
                    episode_id: episode_id.to_string(),
                },
            )));
        }
        if let Some(title_id) = identifier.strip_prefix("title:") {
            return Ok(Some((title_id.to_string(), SubmissionScope::Title)));
        }
        if let Some(link_id) = identifier.strip_prefix("series_movie:") {
            let Some(link) = self
                .services
                .catalog
                .shows
                .get_series_movie_link_by_id(link_id)
                .await?
            else {
                return Ok(None);
            };
            return Ok(Some((
                link.series_title_id,
                SubmissionScope::SeriesMovie {
                    series_movie_link_id: link_id.to_string(),
                },
            )));
        }
        if let Some(collection_id) = identifier.strip_prefix("collection:") {
            let Some(collection) = self
                .services
                .catalog
                .shows
                .get_collection_by_id(collection_id)
                .await?
            else {
                return Ok(None);
            };
            return Ok(Some((
                collection.title_id,
                SubmissionScope::Collection {
                    collection_id: collection_id.to_string(),
                },
            )));
        }
        Ok(None)
    }

    /// Resolve a wanted identifier (state-row id, else convergence scope key) to a
    /// persisted acquisition-state row, creating one if the scope has none yet
    /// (RFC 119 §7.4 — pause/resume must work off a scope key, not only a row id).
    /// Returns the loaded row so callers see its real id/status.
    pub(crate) async fn resolve_or_create_wanted_state_row(
        &self,
        identifier: &str,
    ) -> AppResult<Option<AcquisitionScopeState>> {
        // An existing state-row id resolves directly.
        if let Some(item) = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(identifier.trim())
            .await?
        {
            return Ok(Some(item));
        }

        let Some((title_id, scope)) = self.resolve_scope_identifier(identifier).await? else {
            return Ok(None);
        };
        // Already a row for this scope? (e.g. an episode key whose row exists.)
        let (episode_id, collection_id, series_movie_link_id) = match &scope {
            SubmissionScope::Episode { episode_id } => (Some(episode_id.clone()), None, None),
            SubmissionScope::Collection { collection_id } => {
                (None, Some(collection_id.clone()), None)
            }
            SubmissionScope::SeriesMovie {
                series_movie_link_id,
            } => (None, None, Some(series_movie_link_id.clone())),
            _ => (None, None, None),
        };
        if let Some(existing) = self
            .find_wanted_state_for_scope(
                &title_id,
                episode_id.as_deref(),
                collection_id.as_deref(),
                series_movie_link_id.as_deref(),
            )
            .await?
        {
            return Ok(Some(existing));
        }

        let Some(title) = self.services.catalog.titles.get_by_id(&title_id).await? else {
            return Ok(None);
        };
        let (media_type, season_number) = match &scope {
            SubmissionScope::Episode { episode_id } => {
                let episode = self
                    .services
                    .catalog
                    .shows
                    .get_episode_by_id(episode_id)
                    .await?;
                ("episode", episode.and_then(|episode| episode.season_number))
            }
            SubmissionScope::SeriesMovie { .. } => ("series_movie", Some("0".to_string())),
            SubmissionScope::Collection { .. } => ("episode", None),
            _ => ("movie", None),
        };
        let view = self.new_wanted_state_view(
            &title,
            media_type,
            episode_id,
            collection_id,
            series_movie_link_id,
            season_number,
        );
        let row_id = self
            .services
            .workflow
            .acquisition_scope_states
            .ensure_acquisition_scope_state(&view)
            .await?;
        self.services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(&row_id)
            .await
    }

    async fn acquisition_scope_label(&self, title_id: &str, _scope: &SubmissionScope) -> String {
        self.services
            .catalog
            .titles
            .get_by_id(title_id)
            .await
            .ok()
            .flatten()
            .map(|title| title.name)
            .unwrap_or_else(|| title_id.to_string())
    }

    async fn run_acquisition_search_job(
        &self,
        mut run: JobRunRecord,
        actor: User,
        actor_event: DomainEventActor,
        scopes: Vec<AcquisitionSearchScope>,
        cancellation: tokio_util::sync::CancellationToken,
        _search_guard: tokio::sync::OwnedMutexGuard<()>,
    ) {
        let total = scopes.len();
        let mut processed = 0usize;
        let mut grabbed = 0usize;
        let mut failed = 0usize;
        let mut cancelled = false;

        for scope in scopes {
            if cancellation.is_cancelled() {
                cancelled = true;
                break;
            }
            let _ = self
                .update_acquisition_search_progress(
                    &mut run,
                    AcquisitionSearchProgress {
                        state: "running".to_string(),
                        total,
                        processed,
                        grabbed_count: grabbed,
                        failed_count: failed,
                        current_title: Some(scope.label.clone()),
                    },
                )
                .await;

            // Interactive intent: `queue_best_release` runs the Auto search+grab
            // path (bypasses the background convergence read-gate) and records
            // coverage via the search hook. A search that finds nothing grabbable is
            // a completed search, not a failure.
            match self
                .queue_best_release(
                    &actor,
                    &scope.title_id,
                    scope.scope.clone(),
                    SubmissionConflictPolicy::Skip,
                )
                .await
            {
                Ok(QueueDownloadOutcome::Queued(_)) => grabbed += 1,
                Ok(QueueDownloadOutcome::Conflict(_)) => {}
                Err(AppError::Validation(_)) => {}
                Err(error) => {
                    failed += 1;
                    tracing::warn!(
                        title_id = scope.title_id.as_str(),
                        error = %error,
                        "acquisition search job: scope search failed"
                    );
                }
            }
            processed += 1;
        }

        self.finish_acquisition_search_job(
            run,
            actor_event,
            total,
            processed,
            grabbed,
            failed,
            cancelled,
        )
        .await;
    }

    async fn update_acquisition_search_progress(
        &self,
        run: &mut JobRunRecord,
        progress: AcquisitionSearchProgress,
    ) -> AppResult<()> {
        run.progress_json = serde_json::to_string(&progress).ok();
        run.updated_at = chrono::Utc::now();
        let updated = self.services.events.job_runs.update_job_run(run).await?;
        *run = updated.clone();
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_acquisition_search_job(
        &self,
        mut run: JobRunRecord,
        actor: DomainEventActor,
        total: usize,
        processed: usize,
        grabbed: usize,
        failed: usize,
        cancelled: bool,
    ) {
        let status = if cancelled {
            JobRunStatus::Warning
        } else if failed == 0 {
            JobRunStatus::Completed
        } else if grabbed == 0 && processed == failed {
            JobRunStatus::Failed
        } else {
            JobRunStatus::Warning
        };
        let state = acquisition_search_state_for_status(status, cancelled);
        let completed_at = chrono::Utc::now();
        run.status = status;
        run.progress_json = serde_json::to_string(&AcquisitionSearchProgress {
            state: state.to_string(),
            total,
            processed,
            grabbed_count: grabbed,
            failed_count: failed,
            current_title: None,
        })
        .ok();
        run.summary_text = Some(if cancelled {
            format!("Acquisition search cancelled after {processed} scope(s); grabbed {grabbed}")
        } else {
            format!("Searched {processed} scope(s); grabbed {grabbed}, failed {failed}")
        });
        run.error_text =
            (status == JobRunStatus::Failed).then(|| "all acquisition searches failed".to_string());
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;

        match self.services.events.job_runs.update_job_run(&run).await {
            Ok(updated) => {
                self.runtime
                    .jobs
                    .job_run_tracker
                    .upsert_active_run(JobRun::from_record(&updated, None))
                    .await;
                let payload = if status == JobRunStatus::Failed {
                    DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                        run_id: updated.id.clone(),
                        job_key: updated.job_key.as_str().to_string(),
                        error_text: updated.error_text.clone(),
                    })
                } else {
                    DomainEventPayload::JobRunCompleted(JobRunCompletedEventData {
                        run_id: updated.id.clone(),
                        job_key: updated.job_key.as_str().to_string(),
                        summary_text: updated.summary_text.clone(),
                    })
                };
                let _ = self
                    .append_domain_event(crate::domain_events::new_job_run_domain_event(
                        actor,
                        updated.id.clone(),
                        payload,
                    ))
                    .await;
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to finish acquisition search job");
            }
        }

        self.runtime
            .acquisition
            .acquisition_search_cancellation_tokens
            .lock()
            .await
            .remove(&run.id);
    }

    /// The current state of an interactive acquisition-search job (RFC 119 §7.3),
    /// for the `acquisitionSearchJob` query. Visible to any actor who may read job
    /// runs (`ManageSystemSettings`, matching the jobs surface).
    pub async fn acquisition_search_job(
        &self,
        actor: &User,
        run_id: &str,
    ) -> AppResult<Option<AcquisitionSearchJobView>> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let Some(run) = self.services.events.job_runs.get_job_run(run_id).await? else {
            return Ok(None);
        };
        if run.job_key != JobKey::AcquisitionSearch {
            return Ok(None);
        }
        Ok(Some(self.acquisition_search_job_view(&run)))
    }

    fn acquisition_search_job_view(&self, run: &JobRunRecord) -> AcquisitionSearchJobView {
        let progress = run
            .progress_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<AcquisitionSearchProgress>(json).ok());
        let state = progress
            .as_ref()
            .map(|progress| progress.state.clone())
            .unwrap_or_else(|| acquisition_search_state_for_status(run.status, false).to_string());
        AcquisitionSearchJobView {
            id: run.id.clone(),
            state,
            total: progress.as_ref().map(|p| p.total as i32).unwrap_or(0),
            processed: progress.as_ref().map(|p| p.processed as i32).unwrap_or(0),
            grabbed_count: progress
                .as_ref()
                .map(|p| p.grabbed_count as i32)
                .unwrap_or(0),
            failed_count: progress
                .as_ref()
                .map(|p| p.failed_count as i32)
                .unwrap_or(0),
            current_title: progress.and_then(|p| p.current_title),
            started_at: run.started_at.to_rfc3339(),
            finished_at: run.completed_at.map(|at| at.to_rfc3339()),
        }
    }

    /// Cancel a running interactive acquisition-search job (RFC 119 §7.3). Requires
    /// `ManageSystemSettings` (the jobs surface); signals the job's cancellation
    /// token so it stops between scopes. Returns whether a running job was signalled.
    pub async fn cancel_acquisition_search(&self, actor: &User, run_id: &str) -> AppResult<bool> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let token = self
            .runtime
            .acquisition
            .acquisition_search_cancellation_tokens
            .lock()
            .await
            .get(run_id)
            .cloned();
        match token {
            Some(token) => {
                token.cancel();
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

/// The best-release search scope for a derived view row. Episodes/movies/series
/// movies map to their single-scope submission target; collection/pack rows are
/// not individually searchable by this job (handled by the cursor), so they are
/// skipped.
fn submission_scope_for_view(view: &WantedScopeView) -> Option<SubmissionScope> {
    match view.media_type.as_str() {
        "episode" => view
            .episode_id
            .clone()
            .map(|episode_id| SubmissionScope::Episode { episode_id }),
        "series_movie" => view
            .series_movie_link_id
            .clone()
            .map(|series_movie_link_id| SubmissionScope::SeriesMovie {
                series_movie_link_id,
            }),
        "movie" => Some(SubmissionScope::Title),
        _ => None,
    }
}

/// Deterministic order: title name, then numeric season, then numeric episode —
/// the same ordering the cutoff-unmet view uses.
fn sort_wanted_views(rows: &mut [WantedScopeView]) {
    rows.sort_by(|left, right| {
        left.title_name
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .cmp(
                &right
                    .title_name
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
            )
            .then_with(|| {
                parse_sort_number(left.season_number.as_deref())
                    .cmp(&parse_sort_number(right.season_number.as_deref()))
            })
            .then_with(|| {
                parse_sort_number(left.episode_number.as_deref())
                    .cmp(&parse_sort_number(right.episode_number.as_deref()))
            })
            .then_with(|| left.scope_key.cmp(&right.scope_key))
    });
}
