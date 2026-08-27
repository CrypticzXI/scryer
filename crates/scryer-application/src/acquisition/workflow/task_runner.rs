/// Scheduler value hint for a hot acquisition target (recent air/release/add):
/// high value so the scope is processed promptly and keeps admitting
/// even while the account's API quota is under pressure. Equals the neutral
/// baseline, so hot work is never shed by the low-value pressure gate.
const BACKGROUND_HOT_TARGET_VALUE: f64 = 1.0;

/// Scheduler value hint for a cold acquisition target (long-tail / upgrades):
/// low value so the quota-pressure gate drains it first,
/// yielding shared account quota to RSS polls and hot acquisition. Above the
/// absolute `LOW_VALUE_BACKGROUND_THRESHOLD` floor, so a cold scope still
/// admits when quota is healthy — it only defers once quota tightens.
const BACKGROUND_COLD_TARGET_VALUE: f64 = 0.25;

/// Maximum number of titles whose missing-media acquisition pipelines may be
/// evaluated concurrently. Indexer strategy admission is bounded separately.
const BACKGROUND_ACQUISITION_TITLE_LIMIT: usize = 4;

#[derive(Debug, Clone, Copy)]
struct BackgroundAcquisitionSettings {
    max_scopes_per_cycle: usize,
}

impl AppUseCase {
    async fn background_acquisition_settings(&self) -> AppResult<BackgroundAcquisitionSettings> {
        let max_scopes_per_cycle = self
            .read_setting_i64_value(
                crate::acquisition::convergence::ACQUISITION_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE_KEY,
                None,
            )
            .await?
            .filter(|value| *value > 0)
            .unwrap_or(
                crate::acquisition::convergence::DEFAULT_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE,
            ) as usize;
        Ok(BackgroundAcquisitionSettings {
            max_scopes_per_cycle,
        })
    }
}

async fn blocked_acquisition_facets_after_quiet_wait(app: &AppUseCase) -> Vec<MediaFacet> {
    let blocked_facets = app
        .runtime
        .library
        .library_scan_tracker
        .active_facets()
        .await;
    if blocked_facets.is_empty() {
        return Vec::new();
    }

    metrics::counter!("scryer_background_acquisition_scan_owned_yields_total").increment(1);
    debug!(
        blocked_facets = ?active_scan_facet_labels(&blocked_facets),
        wait_secs = ACQUISITION_SCAN_QUIET_WAIT.as_secs(),
        "background acquisition: yielding while library scan owns active facet"
    );

    let _ = tokio::time::timeout(
        ACQUISITION_SCAN_QUIET_WAIT,
        app.runtime
            .library
            .library_scan_tracker
            .wait_for_active_facets_change(&blocked_facets),
    )
    .await;

    let blocked_facets = app
        .runtime
        .library
        .library_scan_tracker
        .active_facets()
        .await;

    if !blocked_facets.is_empty() {
        debug!(
            blocked_facets = ?active_scan_facet_labels(&blocked_facets),
            "background acquisition: deferring due wanted items for actively scanning facets"
        );
    }

    blocked_facets
}
/// Run one background acquisition cycle: recover failed downloads, derive the
/// missing-media target set, rotate the cursor, and process at most four titles
/// concurrently. Fingerprinted convergence coverage only decides which indexer
/// corpus searches may be skipped; it is not the activity being scheduled.
async fn run_background_acquisition_cycle(app: &AppUseCase) {
    let blocked_facets = blocked_acquisition_facets_after_quiet_wait(app).await;
    run_background_acquisition_cycle_with_blocked_facets(app, &blocked_facets).await;
}

pub(crate) async fn run_background_acquisition_cycle_with_blocked_facets(
    app: &AppUseCase,
    blocked_facets: &[MediaFacet],
) {
    prune_standby_candidates(app).await;

    // Failed downloads first: each failure blocklists its release and re-opens
    // its scope under the existing coverage, so this cycle's derivation already
    // sees it — and the scope's saved search results are tried below before
    // any indexer is queried.
    let dl_snapshot = DownloadClientSnapshot::fetch(app).await;
    check_grabbed_for_failures(app, &dl_snapshot).await;

    let now = Utc::now();
    let settings = match app.background_acquisition_settings().await {
        Ok(settings) => settings,
        Err(err) => {
            warn!(error = %err, "failed to load background acquisition settings, skipping cycle");
            return;
        }
    };

    let mut targets = match app.derive_acquisition_targets(&now).await {
        Ok(targets) => targets,
        Err(err) => {
            warn!(error = %err, "failed to derive acquisition targets");
            return;
        }
    };
    if !blocked_facets.is_empty() {
        targets.retain(|target| !blocked_facets.contains(&target.facet));
    }
    if targets.is_empty() {
        return;
    }

    if !has_enabled_download_clients(app).await {
        warn!(
            target_count = targets.len(),
            "background acquisition: no enabled download clients configured, skipping cycle"
        );
        return;
    }

    let resume = app.background_acquisition_resume_position().await;
    let max_scopes = settings.max_scopes_per_cycle.max(1);
    let selection = crate::acquisition::targets::select_background_acquisition_batch(
        &targets,
        resume.as_deref(),
        max_scopes,
    );
    app.store_background_acquisition_resume_position(selection.resume_after.as_deref())
        .await;
    if selection.indices.is_empty() {
        return;
    }

    debug!(
        target_count = targets.len(),
        selected_count = selection.indices.len(),
        "background acquisition cycle: evaluating missing scopes"
    );

    // Scheduler availability, resolved once per cycle for the pre-skip.
    let availability = app.scheduler_availability().await;
    let indexer_hosts = app.indexer_scheduler_host_keys().await;

    let cycle = Arc::new(BackgroundAcquisitionCycleCoordinator::default());

    // Count selected episode scopes per (title_id, season_num). Season pack
    // search is only worthwhile when >= 2 episodes from the same season are in
    // this cycle — mirroring Sonarr's "count > 1 missing" rule before issuing a
    // SeasonSearchCriteria.
    let mut season_due_counts: std::collections::HashMap<(String, u32), usize> =
        std::collections::HashMap::new();
    for index in &selection.indices {
        let target = &targets[*index];
        if target.media_type == "episode"
            && let Some(sn) = target.season_number.as_deref()
            && let Ok(n) = sn.parse::<u32>()
            && n > 0
        {
            *season_due_counts
                .entry((target.title_id.clone(), n))
                .or_insert(0) += 1;
        }
    }

    let mut ready_titles = build_background_acquisition_title_work(&targets, &selection.indices);
    let title_ids = ready_titles
        .iter()
        .map(|work| work.title_id.clone())
        .collect::<Vec<_>>();
    let titles_by_id = match app.services.catalog.titles.get_by_ids(&title_ids).await {
        Ok(titles) => titles
            .into_iter()
            .map(|title| (title.id.clone(), title))
            .collect::<HashMap<_, _>>(),
        Err(error) => {
            warn!(error = %error, "background acquisition: failed to load selected titles");
            return;
        }
    };
    let mut in_flight = FuturesUnordered::new();
    let availability = &availability;
    let indexer_hosts = &indexer_hosts;
    let season_due_counts = &season_due_counts;
    let dl_snapshot = &dl_snapshot;
    let now = &now;
    let targets = &targets;

    debug!(
        selected_count = selection.indices.len(),
        title_count = ready_titles.len(),
        title_limit = BACKGROUND_ACQUISITION_TITLE_LIMIT,
        "background acquisition cycle: dispatching title work"
    );

    loop {
        while in_flight.len() < BACKGROUND_ACQUISITION_TITLE_LIMIT {
            let Some(title_work) = ready_titles.pop_front() else {
                break;
            };
            let Some(title) = titles_by_id.get(&title_work.title_id).cloned() else {
                warn!(
                    title_id = title_work.title_id.as_str(),
                    "background acquisition target references missing title"
                );
                continue;
            };
            let cycle = Arc::clone(&cycle);
            debug!(
                title_id = title_work.title_id.as_str(),
                queued_titles = ready_titles.len(),
                active_titles = in_flight.len() + 1,
                "background acquisition title work started"
            );
            in_flight.push(async move {
                let title_id = title_work.title_id.clone();
                let result = process_background_acquisition_title(
                    app,
                    title,
                    title_work,
                    &targets,
                    now,
                    availability,
                    indexer_hosts,
                    &cycle,
                    season_due_counts,
                    dl_snapshot,
                )
                .await;
                (title_id, result)
            });
        }

        let Some((title_id, result)) = in_flight.next().await else {
            break;
        };
        if let Err(err) = result {
            warn!(
                title_id = title_id.as_str(),
                error = %err,
                "failed to process background acquisition title"
            );
            metrics::counter!("scryer_background_acquisition_title_work_total", "outcome" => "failed")
                .increment(1);
        } else {
            metrics::counter!("scryer_background_acquisition_title_work_total", "outcome" => "completed")
                .increment(1);
        }
    }
}
/// Whether an in-flight submission should stop this scope being searched again.
///
/// `scope_is_occupied` says whether a primary file already sits in the scope. It
/// separates the two cases the last clause cares about: a completed download for
/// an *empty* scope is still on its way to becoming a file, so searching again
/// would duplicate it; a completed download for an *occupied* scope has already
/// resolved one way or the other, and an upgrade search may proceed.
///
/// This used to read `wanted_items.current_score.is_none()` — a score standing in
/// for "has anything landed here", which is the only honest thing that column
/// ever said, and only in one of its five states.
fn submission_blocks_search_for_wanted_item(
    submission: &DownloadSubmission,
    item: &AcquisitionScopeState,
    episode_collection_id: Option<&str>,
    dl_snapshot: &DownloadClientSnapshot,
    tracked_state: Option<scryer_domain::TrackedDownloadState>,
    scope_is_occupied: bool,
) -> bool {
    if !submission_blocks_wanted_item(submission, item, episode_collection_id) {
        return false;
    }

    if tracked_state == Some(scryer_domain::TrackedDownloadState::Failed) {
        return false;
    }

    // **A failure the handler has not processed yet.** The scope is about to be
    // reopened or blocklisted by `handle_failed_downloads`; searching it now
    // races that, and the release it would find is very likely the one that just
    // failed. Sonarr excludes `FailedPending` from `QueueSpecification` for the
    // same reason — it wants the failure resolved first, not the scope frozen.
    if tracked_state
        .is_some_and(|state| matches!(state, scryer_domain::TrackedDownloadState::FailedPending))
    {
        return true;
    }

    // An unobservable queue reads as "possibly active" everywhere else too
    // (`DownloadClientSnapshot::is_active`); with no way to build honest queued
    // pseudo-incumbents, the old whole-scope skip is the safe answer.
    if dl_snapshot.queue_listing_failed() {
        return true;
    }

    // Everything genuinely in flight — `Downloading | ImportPending | Importing`,
    // `ImportBlocked`, or active in the client — is **not** a skip any more. It
    // becomes a queued pseudo-incumbent on the admission ladder (D18), so a
    // better release can still be grabbed over a slow or stuck one while an
    // equal-or-worse one is refused with a reason that says so.

    // A completed download for a scope with nothing in it is on its way to
    // becoming that file. Searching again would fetch the same episode twice,
    // and there is nothing queued left to compare against — it has left the
    // queue. For an *occupied* scope the download has already resolved one way
    // or the other and an upgrade search may proceed.
    submission_is_completed(submission, dl_snapshot) && !scope_is_occupied
}

impl AppUseCase {
    #[cfg(test)]
    pub(crate) async fn run_background_acquisition_cycle_once(&self) {
        run_background_acquisition_cycle(self).await;
    }
}

#[derive(Default)]
struct BackgroundAcquisitionCycleCoordinator {
    state: Mutex<BackgroundAcquisitionCycleState>,
}

#[derive(Default)]
struct BackgroundAcquisitionCycleState {
    attempted_titles: HashSet<String>,
    claimed_episode_ids: HashSet<String>,
    season_pack_attempted: HashSet<(String, u32)>,
    season_pack_grabbed: HashSet<(String, u32)>,
    season_pack_viable: HashSet<(String, u32)>,
    season_candidates: HashMap<(String, u32), Vec<IndexerSearchResult>>,
    grabbed_urls: HashSet<String>,
    attempted_urls_by_route: Vec<(DownloadRouteKey, String)>,
    failed_routes: Vec<DownloadRouteKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubmissionClaim {
    Granted,
    AlreadySubmitted,
    AlreadyAttempted,
    RouteUnavailable,
}

impl BackgroundAcquisitionCycleCoordinator {
    fn lock(&self) -> std::sync::MutexGuard<'_, BackgroundAcquisitionCycleState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn claimed_episode_ids(&self) -> HashSet<String> {
        self.lock().claimed_episode_ids.clone()
    }

    fn is_episode_claimed(&self, episode_id: &str) -> bool {
        self.lock().claimed_episode_ids.contains(episode_id)
    }

    fn claim_episode_ids<I>(&self, episode_ids: I)
    where
        I: IntoIterator<Item = String>,
    {
        self.lock().claimed_episode_ids.extend(episode_ids);
    }

    fn begin_title_pack(&self, title_id: &str) -> bool {
        self.lock().attempted_titles.insert(title_id.to_string())
    }

    fn complete_title_pack_stage(&self, title_id: &str) {
        self.lock().attempted_titles.insert(title_id.to_string());
    }

    fn begin_season_pack(&self, key: &(String, u32)) -> bool {
        self.lock().season_pack_attempted.insert(key.clone())
    }

    fn complete_season_pack_stage(&self, key: &(String, u32)) {
        self.lock().season_pack_attempted.insert(key.clone());
    }

    fn cache_season_candidates(
        &self,
        key: &(String, u32),
        candidates: impl IntoIterator<Item = IndexerSearchResult>,
    ) {
        let mut state = self.lock();
        let cached = state.season_candidates.entry(key.clone()).or_default();
        for candidate in candidates {
            let duplicate = cached.iter().any(|existing| {
                existing.indexer_id == candidate.indexer_id
                    && existing.guid == candidate.guid
                    && existing.title == candidate.title
            });
            if !duplicate {
                cached.push(candidate);
            }
        }
    }

    fn season_candidates(&self, key: &(String, u32)) -> Vec<IndexerSearchResult> {
        self.lock()
            .season_candidates
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    fn season_pack_grabbed(&self, key: &(String, u32)) -> bool {
        self.lock().season_pack_grabbed.contains(key)
    }

    fn season_pack_viable(&self, key: &(String, u32)) -> bool {
        self.lock().season_pack_viable.contains(key)
    }

    fn mark_season_pack_grabbed(&self, key: &(String, u32)) {
        let mut state = self.lock();
        state.season_pack_grabbed.insert(key.clone());
        state.season_pack_viable.insert(key.clone());
    }

    fn mark_season_pack_viable(&self, key: &(String, u32)) {
        self.lock().season_pack_viable.insert(key.clone());
    }

    fn clear_season_pack_viable(&self, key: &(String, u32)) {
        self.lock().season_pack_viable.remove(key);
    }

    fn failed_routes(&self) -> Vec<DownloadRouteKey> {
        self.lock().failed_routes.clone()
    }

    fn mark_failed_route(&self, route: DownloadRouteKey) {
        let mut state = self.lock();
        if !state.failed_routes.contains(&route) {
            state.failed_routes.push(route);
        }
    }

    fn claim_submission(&self, route: DownloadRouteKey, url: &str) -> SubmissionClaim {
        let mut state = self.lock();
        if state.failed_routes.contains(&route) {
            return SubmissionClaim::RouteUnavailable;
        }
        if state.grabbed_urls.contains(url) {
            return SubmissionClaim::AlreadySubmitted;
        }
        let attempted = (route, url.to_string());
        if state.attempted_urls_by_route.contains(&attempted) {
            return SubmissionClaim::AlreadyAttempted;
        }
        state.attempted_urls_by_route.push(attempted);
        SubmissionClaim::Granted
    }

    fn mark_submitted(&self, url: &str) {
        self.lock().grabbed_urls.insert(url.to_string());
    }
}

#[derive(Clone, Debug)]
enum BackgroundAcquisitionWorkKind {
    TitlePack,
    SeasonPack { season: u32 },
    Scope,
}

#[derive(Clone, Debug)]
struct BackgroundAcquisitionWork {
    target_index: usize,
    kind: BackgroundAcquisitionWorkKind,
}

#[derive(Debug)]
struct BackgroundAcquisitionTitleWork {
    title_id: String,
    ready: VecDeque<BackgroundAcquisitionWork>,
}

fn build_background_acquisition_title_work(
    targets: &[crate::acquisition::targets::AcquisitionTarget],
    selected_indices: &[usize],
) -> VecDeque<BackgroundAcquisitionTitleWork> {
    let mut title_order = Vec::new();
    let mut indices_by_title = HashMap::<String, Vec<usize>>::new();
    for &target_index in selected_indices {
        let title_id = targets[target_index].title_id.clone();
        if !indices_by_title.contains_key(&title_id) {
            title_order.push(title_id.clone());
        }
        indices_by_title
            .entry(title_id)
            .or_default()
            .push(target_index);
    }

    title_order
        .into_iter()
        .filter_map(|title_id| {
            let indices = indices_by_title.remove(&title_id)?;
            let mut ready = VecDeque::new();
            let episode_indices = indices
                .iter()
                .copied()
                .filter(|index| targets[*index].media_type == "episode")
                .collect::<Vec<_>>();
            if let Some(&title_pack_index) = episode_indices.first() {
                ready.push_back(BackgroundAcquisitionWork {
                    target_index: title_pack_index,
                    kind: BackgroundAcquisitionWorkKind::TitlePack,
                });
                let mut seen_seasons = HashSet::new();
                for target_index in episode_indices {
                    let Some(season) = targets[target_index]
                        .season_number
                        .as_deref()
                        .and_then(|value| value.parse::<u32>().ok())
                        .filter(|season| *season > 0)
                    else {
                        continue;
                    };
                    if !seen_seasons.insert(season) || target_index == title_pack_index {
                        continue;
                    }
                    ready.push_back(BackgroundAcquisitionWork {
                        target_index,
                        kind: BackgroundAcquisitionWorkKind::SeasonPack { season },
                    });
                }
            }
            ready.extend(
                indices
                    .into_iter()
                    .map(|target_index| BackgroundAcquisitionWork {
                        target_index,
                        kind: BackgroundAcquisitionWorkKind::Scope,
                    }),
            );
            Some(BackgroundAcquisitionTitleWork { title_id, ready })
        })
        .collect()
}

fn episode_ids_for_scope(scope: &SubmissionScope) -> Option<&[String]> {
    match scope {
        SubmissionScope::EpisodeSet { episode_ids } => Some(episode_ids),
        _ => None,
    }
}

async fn recovered_scope_episode_ids(app: &AppUseCase, scope: &SubmissionScope) -> Vec<String> {
    match scope {
        SubmissionScope::EpisodeSet { episode_ids } => episode_ids.clone(),
        SubmissionScope::Episode { episode_id } => vec![episode_id.clone()],
        SubmissionScope::Collection { collection_id } => match app
            .services
            .catalog
            .shows
            .list_episodes_for_collection(collection_id)
            .await
        {
            Ok(episodes) => episodes.into_iter().map(|episode| episode.id).collect(),
            Err(error) => {
                warn!(
                    collection_id,
                    error = %error,
                    "series-pack search: failed to expand recovered collection coverage"
                );
                Vec::new()
            }
        },
        SubmissionScope::Title | SubmissionScope::SeriesMovie { .. } | SubmissionScope::Orphan => {
            Vec::new()
        }
    }
}

async fn restore_anchor_standby_releases(
    app: &AppUseCase,
    anchor: &AcquisitionScopeState,
    standby_releases: &[PendingRelease],
) {
    let _ = app
        .services
        .workflow
        .pending_releases
        .delete_standby_pending_releases_for_wanted_item(&anchor.id)
        .await;
    for standby in standby_releases {
        if let Err(error) = app
            .services
            .workflow
            .pending_releases
            .insert_pending_release(standby)
            .await
        {
            warn!(
                wanted_item_id = anchor.id.as_str(),
                release = standby.release_title.as_str(),
                error = %error,
                "series-pack search: failed to restore anchor standby candidate"
            );
        }
    }
}

fn is_series_pack_candidate(candidate: &IndexerSearchResult) -> bool {
    candidate
        .parsed_release_metadata
        .as_ref()
        .and_then(|parsed| parsed.episode.as_ref())
        .is_some_and(|episode| episode.is_series_pack)
}

#[expect(
    clippy::too_many_arguments,
    reason = "the one-shot title lookup needs the cycle search state"
)]
async fn try_series_pack_for_title(
    app: &AppUseCase,
    title: &Title,
    search_title: &Title,
    target: &crate::acquisition::targets::AcquisitionTarget,
    now: &DateTime<Utc>,
    availability: &crate::acquisition::convergence::SchedulerAvailability,
    indexer_hosts: &HashMap<String, String>,
    dl_snapshot: &DownloadClientSnapshot,
    failed_routes: &[DownloadRouteKey],
    submissions: &[DownloadSubmission],
    tracked_states: &HashMap<
        crate::contracts::ClientJobLocator,
        scryer_domain::TrackedDownloadState,
    >,
    claimed_episode_ids: &HashSet<String>,
) -> AppResult<Option<Vec<String>>> {
    let mut title_subject = app
        .resolve_release_search_subject_for_title(search_title)
        .await?;
    title_subject.submission_scope = SubmissionScope::Title;
    let episodes = match app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await
    {
        Ok(episodes) => episodes,
        Err(error) => {
            warn!(
                title_id = title.id.as_str(),
                error = %error,
                "series-pack search: failed to load episodes"
            );
            return Ok(None);
        }
    };
    let mut owned_episode_ids = match app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
    {
        Ok(files) => files
            .into_iter()
            .filter(|file| file.role.is_primary())
            .filter_map(|file| file.episode_id)
            .collect::<HashSet<_>>(),
        Err(error) => {
            warn!(
                title_id = title.id.as_str(),
                error = %error,
                "series-pack search: failed to load media ownership"
            );
            return Ok(None);
        }
    };
    owned_episode_ids.extend(
        crate::acquisition_coverage::in_flight_series_pack_episode_ids(
            &episodes,
            submissions,
            tracked_states,
            dl_snapshot,
        ),
    );
    let eligible_collection_ids =
        crate::acquisition_coverage::eligible_series_pack_collection_ids(&episodes);
    if eligible_collection_ids.is_empty()
        || crate::acquisition_coverage::eligible_missing_series_pack_episode_count(
            &episodes,
            &owned_episode_ids,
        ) < 2
        || !crate::acquisition_coverage::title_series_pack_missing_ratio_qualifies(
            &episodes,
            &owned_episode_ids,
        )
    {
        return Ok(None);
    }

    let Some(convergence) = app
        .resolve_series_pack_convergence(search_title, &title_subject, &eligible_collection_ids)
        .await
    else {
        return Ok(None);
    };
    let uncovered = match app
        .uncovered_indexers_for_scope(
            &convergence.scope_key,
            &convergence.facet,
            &convergence.fingerprint,
            &convergence.routed_indexer_ids,
        )
        .await
    {
        Ok(uncovered) => uncovered,
        Err(error) => {
            warn!(
                title_id = title.id.as_str(),
                scope_key = convergence.scope_key.as_str(),
                error = %error,
                "series-pack search: failed to read set coverage; searching all routed indexers"
            );
            convergence.routed_indexer_ids.clone()
        }
    };
    if uncovered.is_empty()
        || !uncovered.iter().any(|indexer_id| {
            availability.indexer_available(
                indexer_hosts.get(indexer_id).map(String::as_str),
                indexer_id,
            )
        })
    {
        return Ok(None);
    }
    let (candidates, fired_indexer_ids) = app
        .search_and_score_subject_restricted_with_fired_indexers(
            search_title,
            &title_subject,
            "background_acquisition_series_pack",
            SearchMode::Auto,
            tokio_util::sync::CancellationToken::new(),
            Some(uncovered.into_iter().collect()),
            Some(if target.is_hot {
                BACKGROUND_HOT_TARGET_VALUE
            } else {
                BACKGROUND_COLD_TARGET_VALUE
            }),
        )
        .await?;

    let (evaluated_candidates, qualifying_collection_ids) = evaluate_series_pack_candidates(
        app,
        title,
        &title_subject,
        candidates,
        &episodes,
        &owned_episode_ids,
        claimed_episode_ids,
    )
    .await;

    if evaluated_candidates.is_empty() {
        record_series_pack_search_coverage(
            app,
            &convergence,
            &fired_indexer_ids,
            &qualifying_collection_ids,
        )
        .await;
        return Ok(None);
    }

    let anchors =
        series_pack_candidate_anchors(app, title, &evaluated_candidates, &episodes).await?;
    record_series_pack_search_coverage(
        app,
        &convergence,
        &fired_indexer_ids,
        &qualifying_collection_ids,
    )
    .await;
    let blocklist = app.load_title_release_blocklist_signatures(&title.id).await;

    for (candidate_index, candidate) in evaluated_candidates.iter().enumerate() {
        let key = crate::app_usecase_discovery::release_search_key(candidate);
        let Some(anchor) = anchors.get(&key) else {
            continue;
        };
        let preserved_standby = match app
            .services
            .workflow
            .pending_releases
            .list_standby_pending_releases_for_wanted_item(&anchor.id)
            .await
        {
            Ok(standby) => standby,
            Err(error) => {
                warn!(
                    wanted_item_id = anchor.id.as_str(),
                    error = %error,
                    "series-pack search: failed to snapshot anchor standby candidates"
                );
                continue;
            }
        };
        if !persist_standby_candidates(
            app,
            anchor,
            title,
            &evaluated_candidates,
            candidate_index,
            now,
            failed_routes,
            &blocklist,
            |saved| crate::app_usecase_discovery::release_search_key(saved) == key,
        )
        .await
        {
            restore_anchor_standby_releases(app, anchor, &preserved_standby).await;
            continue;
        }

        let Some(candidate_scope) = candidate.parsed_release_metadata.as_ref().map(|parsed| {
            crate::acquisition_coverage::resolve_release_coverage(parsed, &episodes, &[], None)
                .submission_scope()
        }) else {
            warn!(
                release = candidate.title.as_str(),
                "series-pack search: evaluated candidate lost parsed metadata"
            );
            restore_anchor_standby_releases(app, anchor, &preserved_standby).await;
            continue;
        };
        let outcome = try_saved_candidates(
            app,
            anchor,
            None,
            Some(claimed_episode_ids),
            dl_snapshot,
            now,
        )
        .await;
        let (scope, standby_start, recovered) = match outcome {
            StandbyRecoveryOutcome::Recovered { scope } => (Some(scope), candidate_index + 1, true),
            StandbyRecoveryOutcome::Active { scope } => (Some(scope), candidate_index + 1, false),
            StandbyRecoveryOutcome::Deferred { scope } => (scope, candidate_index, false),
            StandbyRecoveryOutcome::Parked { scope } => {
                let candidate_is_parked = scope.as_ref() == Some(&candidate_scope);
                (
                    scope,
                    if candidate_is_parked {
                        candidate_index + 1
                    } else {
                        candidate_index
                    },
                    false,
                )
            }
            StandbyRecoveryOutcome::Exhausted { .. } => {
                restore_anchor_standby_releases(app, anchor, &preserved_standby).await;
                continue;
            }
        };
        if recovered {
            persist_series_pack_runner_ups(
                app,
                title,
                &evaluated_candidates,
                standby_start,
                &anchors,
                now,
                failed_routes,
                &blocklist,
            )
            .await;
        } else {
            restore_anchor_standby_releases(app, anchor, &preserved_standby).await;
        }
        return Ok(match scope {
            Some(scope) => {
                let episode_ids = recovered_scope_episode_ids(app, &scope).await;
                (!episode_ids.is_empty()).then_some(episode_ids)
            }
            None => None,
        });
    }

    Ok(None)
}

async fn evaluate_series_pack_candidates(
    app: &AppUseCase,
    title: &Title,
    title_subject: &crate::acquisition_release_search::ResolvedReleaseSearchSubject,
    candidates: Vec<IndexerSearchResult>,
    episodes: &[Episode],
    owned_episode_ids: &HashSet<String>,
    claimed_episode_ids: &HashSet<String>,
) -> (Vec<IndexerSearchResult>, HashSet<String>) {
    let mut groups = HashMap::<Vec<String>, Vec<(usize, IndexerSearchResult)>>::new();
    let mut collection_ids = HashSet::new();

    for (rank, candidate) in candidates.into_iter().enumerate() {
        if !is_series_pack_candidate(&candidate) {
            continue;
        }
        let Some(parsed) = candidate.parsed_release_metadata.as_ref() else {
            continue;
        };
        if !crate::acquisition_coverage::series_pack_missing_ratio_qualifies(
            parsed,
            episodes,
            owned_episode_ids,
        ) {
            continue;
        }

        collection_ids.extend(crate::acquisition_coverage::series_pack_collection_ids(
            parsed, episodes,
        ));
        let scope =
            crate::acquisition_coverage::resolve_release_coverage(parsed, episodes, &[], None)
                .submission_scope();
        let Some(mut episode_ids) = episode_ids_for_scope(&scope).map(<[String]>::to_vec) else {
            continue;
        };
        episode_ids.sort();
        episode_ids.dedup();
        if episode_ids
            .iter()
            .any(|episode_id| claimed_episode_ids.contains(episode_id))
        {
            continue;
        }
        groups
            .entry(episode_ids)
            .or_default()
            .push((rank, candidate));
    }

    let mut evaluated = Vec::new();
    for (episode_ids, ranked_candidates) in groups {
        let mut ranks_by_key = HashMap::new();
        let mut candidates = Vec::with_capacity(ranked_candidates.len());
        for (rank, candidate) in ranked_candidates {
            ranks_by_key.insert(
                crate::app_usecase_discovery::release_search_key(&candidate),
                rank,
            );
            candidates.push(candidate);
        }

        let mut scoped_subject = title_subject.clone();
        scoped_subject.submission_scope = SubmissionScope::EpisodeSet { episode_ids };
        for candidate in app
            .evaluate_search_results_for_subject(title, &scoped_subject, candidates, false)
            .await
        {
            let key = crate::app_usecase_discovery::release_search_key(&candidate);
            if let Some(rank) = ranks_by_key.remove(&key) {
                evaluated.push((rank, candidate));
            }
        }
    }

    evaluated.sort_by_key(|(rank, _)| *rank);
    (
        evaluated
            .into_iter()
            .filter(|(_, candidate)| {
                matches!(
                    annotated_auto_decision_code(candidate),
                    ReleaseAutoDecisionCode::Eligible
                        | ReleaseAutoDecisionCode::PendingDelay
                        | ReleaseAutoDecisionCode::AlreadyActive
                )
            })
            .map(|(_, candidate)| candidate)
            .collect(),
        collection_ids,
    )
}

async fn series_pack_candidate_anchors(
    app: &AppUseCase,
    title: &Title,
    candidates: &[IndexerSearchResult],
    episodes: &[Episode],
) -> AppResult<HashMap<String, AcquisitionScopeState>> {
    let states = app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states_for_title_ids(std::slice::from_ref(&title.id))
        .await?;
    let mut states_by_episode = states
        .into_iter()
        .filter_map(|state| {
            state
                .episode_id
                .clone()
                .map(|episode_id| (episode_id, state))
        })
        .collect::<HashMap<_, _>>();
    let mut anchors = HashMap::new();

    for candidate in candidates {
        let Some(parsed) = candidate.parsed_release_metadata.as_ref() else {
            continue;
        };
        let scope =
            crate::acquisition_coverage::resolve_release_coverage(parsed, episodes, &[], None)
                .submission_scope();
        let Some(anchor_episode_id) =
            episode_ids_for_scope(&scope).and_then(|episode_ids| episode_ids.first())
        else {
            continue;
        };
        let anchor = if let Some(anchor) = states_by_episode.get(anchor_episode_id).cloned() {
            anchor
        } else {
            let Some(anchor_episode) = episodes
                .iter()
                .find(|episode| episode.id == *anchor_episode_id)
            else {
                continue;
            };
            let mut anchor = app.new_wanted_state_view(
                title,
                "episode",
                Some(anchor_episode.id.clone()),
                anchor_episode.collection_id.clone(),
                None,
                anchor_episode.season_number.clone(),
            );
            anchor.id = app
                .services
                .workflow
                .acquisition_scope_states
                .ensure_acquisition_scope_state(&anchor)
                .await?;
            states_by_episode.insert(anchor_episode.id.clone(), anchor.clone());
            anchor
        };
        anchors.insert(
            crate::app_usecase_discovery::release_search_key(candidate),
            anchor,
        );
    }

    Ok(anchors)
}

#[expect(
    clippy::too_many_arguments,
    reason = "series-pack runner-ups retain their exact covered anchor and global rank"
)]
async fn persist_series_pack_runner_ups(
    app: &AppUseCase,
    title: &Title,
    candidates: &[IndexerSearchResult],
    start_index: usize,
    anchors: &HashMap<String, AcquisitionScopeState>,
    now: &DateTime<Utc>,
    failed_routes: &[DownloadRouteKey],
    blocklist: &crate::app_usecase_discovery::TitleReleaseBlocklistSignatures,
) {
    let mut anchor_ids = candidates
        .iter()
        .skip(start_index)
        .filter_map(|candidate| {
            anchors.get(&crate::app_usecase_discovery::release_search_key(candidate))
        })
        .map(|anchor| anchor.id.clone())
        .collect::<Vec<_>>();
    anchor_ids.sort();
    anchor_ids.dedup();

    for anchor_id in anchor_ids {
        let Some(anchor) = anchors.values().find(|anchor| anchor.id == anchor_id) else {
            continue;
        };
        persist_standby_candidates(
            app,
            anchor,
            title,
            candidates,
            start_index,
            now,
            failed_routes,
            blocklist,
            |candidate| {
                anchors
                    .get(&crate::app_usecase_discovery::release_search_key(candidate))
                    .is_some_and(|candidate_anchor| candidate_anchor.id == anchor_id)
            },
        )
        .await;
    }
}

async fn record_series_pack_search_coverage(
    app: &AppUseCase,
    convergence: &crate::acquisition::convergence::ScopeConvergence,
    fired_indexer_ids: &[String],
    collection_ids: &HashSet<String>,
) {
    app.record_convergence_coverage(convergence, fired_indexer_ids)
        .await;
    for collection_id in collection_ids {
        let Some(scope_key) =
            crate::acquisition::convergence::series_pack_collection_scope_key(collection_id)
        else {
            continue;
        };
        let mut collection_convergence = convergence.clone();
        collection_convergence.scope_key = scope_key;
        app.record_convergence_coverage(&collection_convergence, fired_indexer_ids)
            .await;
    }
}

struct BackgroundAcquisitionTitleContext {
    title: scryer_domain::Title,
    episodes_by_id: HashMap<String, scryer_domain::Episode>,
    submissions: Vec<DownloadSubmission>,
    tracked_states:
        HashMap<crate::contracts::ClientJobLocator, scryer_domain::TrackedDownloadState>,
}

impl BackgroundAcquisitionTitleContext {
    async fn load(app: &AppUseCase, title: scryer_domain::Title) -> AppResult<Self> {
        let episodes = app
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await?;
        let submission_guard = app
            .runtime
            .acquisition
            .download_submission_guards
            .acquire_title(&title.id)
            .await;
        let submissions = app
            .services
            .workflow
            .download_submissions
            .list_for_title(&title.id)
            .await?;
        if app
            .services
            .workflow
            .download_submissions
            .list_active_unbound_for_title(&title.id)
            .await?
            .is_empty()
        {
            app.runtime
                .acquisition
                .download_submission_guards
                .prime_title_state(&title.id, submissions.clone(), episodes.clone());
        } else {
            app.runtime
                .acquisition
                .download_submission_guards
                .clear_title_state(&title.id);
        }
        drop(submission_guard);
        let submission_identities = submissions
            .iter()
            .map(crate::contracts::ClientJobLocator::from_submission)
            .collect::<Vec<_>>();
        let tracked_states = app
            .services
            .workflow
            .download_submissions
            .list_identity_tracked_states_for_client_items(&submission_identities)
            .await?
            .into_iter()
            .filter_map(|(identity, state)| {
                scryer_domain::TrackedDownloadState::from_str_opt(&state)
                    .map(|state| (identity, state))
            })
            .collect();

        Ok(Self {
            title,
            episodes_by_id: episodes
                .into_iter()
                .map(|episode| (episode.id.clone(), episode))
                .collect(),
            submissions,
            tracked_states,
        })
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one title coordinator owns the cycle-wide acquisition inputs"
)]
async fn process_background_acquisition_title(
    app: &AppUseCase,
    title: scryer_domain::Title,
    mut title_work: BackgroundAcquisitionTitleWork,
    targets: &[crate::acquisition::targets::AcquisitionTarget],
    now: &DateTime<Utc>,
    availability: &crate::acquisition::convergence::SchedulerAvailability,
    indexer_hosts: &HashMap<String, String>,
    cycle: &BackgroundAcquisitionCycleCoordinator,
    season_due_counts: &HashMap<(String, u32), usize>,
    dl_snapshot: &DownloadClientSnapshot,
) -> AppResult<usize> {
    let context = BackgroundAcquisitionTitleContext::load(app, title).await?;
    let mut completed = 0usize;

    while let Some(work) = title_work.ready.pop_front() {
        let target = &targets[work.target_index];
        let pack_stage_only = !matches!(work.kind, BackgroundAcquisitionWorkKind::Scope);
        debug!(
            title_id = title_work.title_id.as_str(),
            scope_key = target.scope_key.as_str(),
            work = ?work.kind,
            "background acquisition target work started"
        );
        if let Err(error) = process_single_target(
            app,
            target,
            now,
            availability,
            indexer_hosts,
            cycle,
            season_due_counts,
            dl_snapshot,
            &context,
            pack_stage_only,
        )
        .await
        {
            warn!(
                scope_key = target.scope_key.as_str(),
                title_id = target.title_id.as_str(),
                error = %error,
                "failed to process background acquisition target"
            );
            metrics::counter!("scryer_background_acquisition_target_work_total", "outcome" => "failed")
                .increment(1);
        } else {
            metrics::counter!("scryer_background_acquisition_target_work_total", "outcome" => "completed")
                .increment(1);
        }

        match work.kind {
            BackgroundAcquisitionWorkKind::TitlePack => {
                cycle.complete_title_pack_stage(&title_work.title_id);
                if let Some(season) = target
                    .season_number
                    .as_deref()
                    .and_then(|value| value.parse::<u32>().ok())
                {
                    cycle.complete_season_pack_stage(&(title_work.title_id.clone(), season));
                }
            }
            BackgroundAcquisitionWorkKind::SeasonPack { season } => {
                cycle.complete_season_pack_stage(&(title_work.title_id.clone(), season));
            }
            BackgroundAcquisitionWorkKind::Scope => {}
        }

        completed += 1;
        if completed.is_multiple_of(ACQUISITION_SLICE_YIELD_INTERVAL) {
            tokio::task::yield_now().await;
        }
    }

    Ok(completed)
}

#[expect(
    clippy::too_many_arguments,
    reason = "target processing coordinates shared acquisition state across a title pass"
)]
async fn process_single_target(
    app: &AppUseCase,
    target: &crate::acquisition::targets::AcquisitionTarget,
    now: &DateTime<Utc>,
    availability: &crate::acquisition::convergence::SchedulerAvailability,
    indexer_hosts: &std::collections::HashMap<String, String>,
    cycle: &BackgroundAcquisitionCycleCoordinator,
    season_due_counts: &std::collections::HashMap<(String, u32), usize>,
    dl_snapshot: &DownloadClientSnapshot,
    context: &BackgroundAcquisitionTitleContext,
    pack_stage_only: bool,
) -> AppResult<()> {
    let title = &context.title;

    // Load episode data for episode-scoped targets
    let episode = if target.media_type == "episode" {
        target
            .episode_id
            .as_deref()
            .and_then(|episode_id| context.episodes_by_id.get(episode_id))
            .cloned()
    } else {
        None
    };
    let effective_collection_id = target
        .collection_id
        .clone()
        .or_else(|| episode.as_ref().and_then(|ep| ep.collection_id.clone()));
    if episode
        .as_ref()
        .is_some_and(|episode| cycle.is_episode_claimed(&episode.id))
    {
        return Ok(());
    }

    // The scope's acquisition-state row, or an unpersisted view when nothing
    // has happened to the scope yet — persisted the moment it is actually
    // searched, so decisions and grabs have their anchor.
    let mut item = match app
        .find_wanted_state_for_scope(
            &target.title_id,
            target.episode_id.as_deref(),
            target.collection_id.as_deref(),
            target.series_movie_link_id.as_deref(),
        )
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => app.new_wanted_state_view(
            &title,
            &target.media_type,
            target.episode_id.clone(),
            effective_collection_id.clone(),
            target.series_movie_link_id.clone(),
            target.season_number.clone(),
        ),
        Err(err) => {
            warn!(
                scope_key = target.scope_key.as_str(),
                error = %err,
                "failed to load acquisition state for target"
            );
            return Ok(());
        }
    };
    let item = &mut item;

    // Item-aware gate: skip only when an active/recent submission blocks this
    // wanted item, not every sibling episode on the same title.
    let submissions = &context.submissions;
    let tracked_states = &context.tracked_states;
    let episode_collection_id = episode_collection_id_for_wanted_item(item, episode.as_ref());

    let has_blocking_download_submission = submissions.iter().any(|submission| {
        let identity = crate::contracts::ClientJobLocator::from_submission(submission);
        submission_blocks_search_for_wanted_item(
            submission,
            item,
            episode_collection_id.as_deref(),
            dl_snapshot,
            tracked_states.get(&identity).copied(),
            target.occupied,
        )
    });

    if has_blocking_download_submission {
        info!(
            title = title.name.as_str(),
            media_type = item.media_type.as_str(),
            episode_id = item.episode_id.as_deref(),
            collection_id = episode_collection_id
                .as_deref()
                .or(item.collection_id.as_deref()),
            "skipping search — download for this wanted item is already active or completed"
        );
        return Ok(());
    }

    // Saved search results first: a failure never costs an indexer query. A
    // `wanted` scope that still holds ranked results from its last search —
    // the remainder after a grab that later failed — walks them in order,
    // re-judged against the blocklist, the swarm and admission. Only an
    // exhausted list (or a scope that never saved one) reaches the convergence
    // gate below.
    let claimed_episode_ids = cycle.claimed_episode_ids();
    let stale_standby_indexer_ids =
        if item.status == AcquisitionScopeStatus::Wanted && !item.id.is_empty() {
            match try_saved_candidates(
                app,
                item,
                None,
                Some(&claimed_episode_ids),
                dl_snapshot,
                now,
            )
            .await
            {
                StandbyRecoveryOutcome::Recovered { scope }
                | StandbyRecoveryOutcome::Active { scope } => {
                    if let Some(episode_ids) = episode_ids_for_scope(&scope) {
                        cycle.claim_episode_ids(episode_ids.iter().cloned());
                    }
                    if let SubmissionScope::Collection { collection_id } = &scope {
                        if let Ok(episodes) = app
                            .services
                            .catalog
                            .shows
                            .list_episodes_for_collection(collection_id)
                            .await
                        {
                            cycle.claim_episode_ids(episodes.into_iter().map(|episode| episode.id));
                        }
                        if let Some(season) = target
                            .season_number
                            .as_deref()
                            .or(episode
                                .as_ref()
                                .and_then(|episode| episode.season_number.as_deref()))
                            .and_then(|season| season.parse::<u32>().ok())
                        {
                            cycle.mark_season_pack_grabbed(&(title.id.clone(), season));
                        }
                    }
                    info!(
                        title = title.name.as_str(),
                        scope_key = target.scope_key.as_str(),
                        "grabbed the next saved search result; no indexer query spent"
                    );
                    return Ok(());
                }
                StandbyRecoveryOutcome::Deferred { .. } => {
                    info!(
                        title = title.name.as_str(),
                        scope_key = target.scope_key.as_str(),
                        "saved search result kept pending until the download client recovers"
                    );
                    return Ok(());
                }
                StandbyRecoveryOutcome::Parked { .. } => {
                    info!(
                        title = title.name.as_str(),
                        scope_key = target.scope_key.as_str(),
                        "best saved search result is held by its delay profile"
                    );
                    return Ok(());
                }
                StandbyRecoveryOutcome::Exhausted { stale_indexer_ids } => stale_indexer_ids,
            }
        } else {
            Vec::new()
        };

    let search_title = app
        .release_search_title_for_wanted_item(&title, item, episode.as_ref())
        .await;

    let subject = app
        .resolve_release_search_subject_for_wanted_item(
            &title,
            &search_title,
            item,
            episode.as_ref(),
        )
        .await;
    let search_season = subject.season;

    // Exhausting saved results is a recovery action, not a new search. Preserve
    // that contract before either the title or episode lane spends an indexer
    // query.
    if !stale_standby_indexer_ids.is_empty() {
        if let Some(convergence) = app.resolve_scope_convergence(&search_title, &subject).await {
            info!(
                title_id = title.id.as_str(),
                scope_key = convergence.scope_key.as_str(),
                stale_indexer_ids = ?stale_standby_indexer_ids,
                "background acquisition: pruned stale standby coverage; the next cycle will refresh these indexers"
            );
            for indexer_id in stale_standby_indexer_ids {
                app.prune_scope_key_coverage(&convergence.scope_key, Some(&indexer_id))
                    .await;
            }
        }
        return Ok(());
    }

    // One title lookup per cycle discovers a qualifying whole-series or
    // multi-season release before the established season and episode paths.
    if target.media_type == "episode"
        && title.facet != MediaFacet::Movie
        && let Some(target_episode) = episode.as_ref()
        && cycle.begin_title_pack(&title.id)
    {
        let failed_routes = cycle.failed_routes();
        let claimed_episode_ids = cycle.claimed_episode_ids();
        match try_series_pack_for_title(
            app,
            &title,
            &search_title,
            target,
            now,
            availability,
            indexer_hosts,
            dl_snapshot,
            &failed_routes,
            &submissions,
            &tracked_states,
            &claimed_episode_ids,
        )
        .await
        {
            Ok(Some(episode_ids)) => {
                let claims_target = episode_ids.contains(&target_episode.id);
                cycle.claim_episode_ids(episode_ids);
                if claims_target {
                    return Ok(());
                }
            }
            Ok(None) => {}
            Err(error) => {
                warn!(
                    title_id = title.id.as_str(),
                    error = %error,
                    "series-pack title search failed"
                );
            }
        }
    }

    // Pack stages own distinct fingerprints; only the later scope stage may be
    // short-circuited by episode/title coverage.
    let (uncovered, convergence_scope_key) = if pack_stage_only {
        (HashSet::new(), None)
    } else {
        let Some(convergence) = app.resolve_scope_convergence(&search_title, &subject).await else {
            debug!(
                title_id = title.id.as_str(),
                scope_key = target.scope_key.as_str(),
                "background acquisition: scope has no routed indexers, skipping"
            );
            return Ok(());
        };
        let uncovered = match app
            .uncovered_indexers_for_scope(
                &convergence.scope_key,
                &convergence.facet,
                &convergence.fingerprint,
                &convergence.routed_indexer_ids,
            )
            .await
        {
            Ok(uncovered) => uncovered,
            Err(err) => {
                warn!(
                    scope_key = convergence.scope_key.as_str(),
                    error = %err,
                    "failed to read scope coverage; searching all routed indexers"
                );
                convergence.routed_indexer_ids.clone()
            }
        };
        if uncovered.is_empty() {
            debug!(
                title_id = title.id.as_str(),
                title_name = title.name.as_str(),
                media_type = target.media_type.as_str(),
                "background acquisition: scope converged across routed indexers, riding RSS"
            );
            return Ok(());
        }
        // Scheduler pre-skip: every uncovered indexer is cooling down or quota
        // exhausted — spend nothing; the scope stays a target and the cursor
        // returns to it once the scheduler frees capacity.
        if !uncovered.iter().any(|indexer_id| {
            availability.indexer_available(
                indexer_hosts.get(indexer_id).map(String::as_str),
                indexer_id,
            )
        }) {
            debug!(
                title_id = title.id.as_str(),
                scope_key = target.scope_key.as_str(),
                uncovered_count = uncovered.len(),
                "background acquisition: uncovered indexers unavailable this cycle, deferring scope"
            );
            return Ok(());
        }
        (uncovered.into_iter().collect(), Some(convergence.scope_key))
    };

    // The scope is about to be searched — its state row exists from here on,
    // so release decisions and grabs have their anchor.
    item.id = app
        .services
        .workflow
        .acquisition_scope_states
        .ensure_acquisition_scope_state(item)
        .await?;
    let mut failed_routes = cycle.failed_routes();

    // Derive the download client category separately — search_category ("series")
    // is for Newznab query type, download_category ("series") is for NZBGet routing.
    //
    // ── Season pack priority ──────────────────────────────────────────────────
    // For episode wanted items, try a season pack search first. Season packs are
    // a first-class release type on Usenet and are more efficient than individual
    // episodes. Individual episode searches only run if no season pack was found
    // this cycle for this (title, season).
    if target.media_type == "episode"
        && let Some(season_num) = search_season
    {
        let season_key = (title.id.clone(), season_num);

        // Only attempt a season pack search when >= 2 episodes from this season
        // are due this cycle (mirrors Sonarr: count > 1 missing → SeasonSearchCriteria).
        let due_count = season_due_counts.get(&season_key).copied().unwrap_or(0);

        if due_count >= 2 && cycle.begin_season_pack(&season_key) {
            let recent_failed_seasons =
                load_recent_failed_season_pack_seasons_for_title(app, &title.id, now).await;

            if recent_failed_seasons.contains(&season_num) {
                info!(
                    title = title.name.as_str(),
                    season = season_num,
                    cooldown_minutes = FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES,
                    "skipping season-pack search after recent failed season-pack attempt"
                );
            } else {
                // Load season episodes for runtime scoring and upgrade checking.
                let season_episodes = if let Some(ref coll_id) = effective_collection_id {
                    app.services
                        .catalog
                        .shows
                        .list_episodes_for_collection(coll_id)
                        .await
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };

                // Calculate total season runtime for accurate size scoring.
                // A 10-episode × 24-min season should expect ~10× a single episode's size.
                let pack_runtime = if !season_episodes.is_empty() {
                    let ep_count = season_episodes.len().max(1) as i32;
                    let per_ep = title.runtime_minutes.unwrap_or(24);
                    Some(per_ep * ep_count)
                } else {
                    title.runtime_minutes
                };

                let pack_subject = app
                    .resolve_release_search_subject_for_season_pack(
                        &search_title,
                        item,
                        episode.as_ref(),
                        season_num,
                        pack_runtime,
                    )
                    .await?;

                // The pack is its own convergence unit: a
                // converged pack scope rides RSS, an unconverged one is searched
                // against its uncovered subset.
                let pack_uncovered = match app
                    .resolve_scope_convergence(&search_title, &pack_subject)
                    .await
                {
                    Some(pack_convergence) => app
                        .uncovered_indexers_for_scope(
                            &pack_convergence.scope_key,
                            &pack_convergence.facet,
                            &pack_convergence.fingerprint,
                            &pack_convergence.routed_indexer_ids,
                        )
                        .await
                        .ok(),
                    None => None,
                };
                let pack_results = if pack_uncovered
                    .as_ref()
                    .is_some_and(|uncovered| uncovered.is_empty())
                {
                    debug!(
                        title_id = title.id.as_str(),
                        season = season_num,
                        "season pack scope converged, riding RSS"
                    );
                    Vec::new()
                } else {
                    match app
                        .search_and_evaluate_subject_restricted(
                            &search_title,
                            &pack_subject,
                            "background_acquisition_season_pack",
                            SearchMode::Auto,
                            tokio_util::sync::CancellationToken::new(),
                            pack_uncovered.map(|uncovered| uncovered.into_iter().collect()),
                            // The pack shares the target's recency lane (§D3).
                            Some(if target.is_hot {
                                BACKGROUND_HOT_TARGET_VALUE
                            } else {
                                BACKGROUND_COLD_TARGET_VALUE
                            }),
                        )
                        .await
                    {
                        Ok(results) => results,
                        Err(err) => {
                            warn!(
                                title_id = title.id.as_str(),
                                season = season_num,
                                error = %err,
                                "season pack search failed"
                            );
                            Vec::new()
                        }
                    }
                };

                cycle.cache_season_candidates(
                    &season_key,
                    pack_results
                        .iter()
                        .filter(|candidate| {
                            !candidate_is_season_pack_for_season(candidate, season_num)
                        })
                        .cloned(),
                );

                for candidate in pack_results
                    .iter()
                    .filter(|candidate| candidate_is_season_pack_for_season(candidate, season_num))
                {
                    let decision_code = annotated_auto_decision_code(candidate);
                    // Recorded before any gate ran for this pack, so there is no
                    // bar to name.
                    record_release_decision(app, item, &title, candidate, decision_code, None, now)
                        .await;
                    if matches!(
                        decision_code,
                        ReleaseAutoDecisionCode::PendingDelay
                            | ReleaseAutoDecisionCode::AlreadyActive
                    ) {
                        cycle.mark_season_pack_viable(&season_key);
                    }
                }

                'season_pack_candidates: for (best_pack_index, best_pack) in
                    pack_results.iter().enumerate().filter(|(_, candidate)| {
                        candidate_is_season_pack_for_season(candidate, season_num)
                            && candidate.auto_eligible == Some(true)
                    })
                {
                    let pack_route = DownloadRouteKey::for_candidate(best_pack)
                        .expect("candidate route key always exists, including unknown source kind");
                    if failed_routes.contains(&pack_route) {
                        continue;
                    }
                    let pack_url = best_pack
                        .canonical_download_source()
                        .map(|(source, _)| source);
                    let url_str = pack_url.as_deref().unwrap_or("").to_string();
                    if !url_str.is_empty()
                        && matches!(
                            cycle.claim_submission(pack_route.clone(), &url_str),
                            SubmissionClaim::Granted
                        )
                    {
                        let download_cat = app.derive_download_category(&title.facet).await;
                        let is_recent = app.is_recent_for_queue_priority(
                            best_pack
                                .published_at
                                .as_deref()
                                .or(episode.as_ref().and_then(|item| item.air_date.as_deref()))
                                .or(title.first_aired.as_deref())
                                .or(title.digital_release_date.as_deref()),
                        );
                        let pack_title = Some(best_pack.title.clone());
                        let pack_hint = normalize_release_attempt_hint(pack_url.as_deref());
                        let pack_title_norm = normalize_release_name(pack_title.as_deref());
                        let pack_password =
                            normalize_release_password(best_pack.password_hint.as_deref());
                        let request_signature = normalize_release_selection_signature(
                            pack_url.as_deref(),
                            pack_title.as_deref(),
                            best_pack.source_kind,
                        );
                        let info_hash_hint = best_pack.info_hash().map(str::to_string);
                        let seed_minimums =
                            crate::ReleaseSeedMinimums::from_release_extra(&best_pack.extra);
                        let download_id = scryer_domain::download_identity::DownloadId::new();
                        let submission_scope = collection_download_submission_scope_for_wanted_item(
                            item,
                            episode.as_ref(),
                        );

                        let canonical_result = app
                            .submit_canonical_download(CanonicalDownloadSubmissionIntent {
                                request: DownloadClientAddRequest {
                                    title: title.clone(),
                                    search_facet: None,
                                    purpose: crate::DownloadSubmissionPurpose::Standard,
                                    download_id: Some(download_id),
                                    source_hint: pack_url.clone(),
                                    staged_nzb: None,
                                    resolved_download_artifact: None,
                                    source_kind: best_pack.source_kind,
                                    source_title: pack_title.clone(),
                                    source_password: pack_password.clone(),
                                    category: Some(download_cat),
                                    queue_priority: None,
                                    download_directory: None,
                                    release_title: Some(best_pack.title.clone()),
                                    indexer_name: Some(best_pack.source.clone()),
                                    indexer_id: best_pack.indexer_id.clone(),
                                    info_hash_hint: info_hash_hint.clone(),
                                    seed_goal_ratio: None,
                                    seed_goal_seconds: None,
                                    tracker_min_seed_ratio: seed_minimums.min_seed_ratio,
                                    tracker_min_seed_time_minutes: seed_minimums
                                        .min_seed_time_minutes,
                                    season_pack_seed_ratio: seed_minimums.season_pack_seed_ratio,
                                    season_pack_seed_time_minutes: seed_minimums
                                        .season_pack_seed_time_minutes,
                                    is_recent,
                                    season_pack: Some(true),
                                },
                                scope: submission_scope.clone(),
                                conflict_policy: SubmissionConflictPolicy::Skip,
                                request_signature: request_signature.clone(),
                                source_provider_name: Some(best_pack.source.clone()),
                                release_size_bytes: best_pack.size_bytes,
                            })
                            .await;

                        let canonical_submission = match canonical_result {
                            Ok(CanonicalDownloadSubmissionOutcome::Accepted(submission)) => {
                                Ok(submission)
                            }
                            Ok(CanonicalDownloadSubmissionOutcome::Conflict(_)) => {
                                break 'season_pack_candidates;
                            }
                            Err(error) => Err(error),
                        };

                        match canonical_submission {
                            Ok(canonical_submission) => {
                                let grab = canonical_submission.grab;
                                let download_job_id = grab.job_id.clone();
                                let facet_label = serde_json::to_string(&title.facet)
                                    .unwrap_or_else(|_| "\"other\"".to_string())
                                    .trim_matches('"')
                                    .to_string();
                                metrics::counter!("scryer_grabs_total", "indexer" => best_pack.source.clone(), "facet" => facet_label).increment(1);
                                app.record_indexer_grab(
                                    best_pack.indexer_id.as_deref(),
                                    Some(best_pack.source.as_str()),
                                );
                                cycle.mark_submitted(&url_str);
                                cycle.mark_season_pack_grabbed(&season_key);
                                let _ = app
                                    .services
                                    .workflow
                                    .release_attempts
                                    .record_release_attempt(
                                        Some(title.id.clone()),
                                        pack_hint,
                                        pack_title_norm,
                                        ReleaseDownloadAttemptOutcome::Success,
                                        None,
                                        pack_password,
                                    )
                                    .await;
                                let mut grabbed_episode_ids = match &submission_scope {
                                    SubmissionScope::Episode { episode_id } => {
                                        vec![episode_id.clone()]
                                    }
                                    SubmissionScope::EpisodeSet { episode_ids } => {
                                        episode_ids.clone()
                                    }
                                    SubmissionScope::Collection { collection_id } => app
                                        .services
                                        .catalog
                                        .shows
                                        .list_episodes_for_collection(collection_id)
                                        .await
                                        .map(|episodes| {
                                            episodes.into_iter().map(|episode| episode.id).collect()
                                        })
                                        .unwrap_or_default(),
                                    SubmissionScope::Title
                                    | SubmissionScope::SeriesMovie { .. }
                                    | SubmissionScope::Orphan => Vec::new(),
                                };
                                grabbed_episode_ids.sort();
                                grabbed_episode_ids.dedup();
                                let covered_wanted_item_ids = app
                                    .covered_wanted_item_ids_for_submission_scope(
                                        &title.id,
                                        &submission_scope,
                                        &item.id,
                                    )
                                    .await?;
                                let grabbed_json = serde_json::json!({
                                    "title": best_pack.title,
                                    "score": best_pack
                                        .quality_profile_decision
                                        .as_ref()
                                        .map(|decision| decision.preference_score)
                                        .unwrap_or(0),
                                    "grabbed_at": now.to_rfc3339(),
                                    "season_pack": true,
                                    "source_provider": best_pack.source.clone(),
                                })
                                .to_string();
                                app.services
                                    .workflow
                                    .acquisition_state
                                    .commit_successful_grab(&SuccessfulGrabCommit {
                                        wanted_item_id: item.id.clone(),
                                        covered_wanted_item_ids,
                                        grabbed_release: grabbed_json,
                                        last_search_at: Some(now.to_rfc3339()),
                                        grabbed_pending_release_id: None,
                                        grabbed_at: Some(now.to_rfc3339()),
                                    })
                                    .await?;
                                let pack_blocklist =
                                    app.load_title_release_blocklist_signatures(&title.id).await;
                                persist_standby_candidates(
                                    app,
                                    item,
                                    &title,
                                    &pack_results,
                                    best_pack_index + 1,
                                    now,
                                    &failed_routes,
                                    &pack_blocklist,
                                    |candidate| {
                                        candidate_is_season_pack_for_season(candidate, season_num)
                                    },
                                )
                                .await;
                                let pack_score = best_pack
                                    .quality_profile_decision
                                    .as_ref()
                                    .map(|d| d.preference_score)
                                    .unwrap_or(0);
                                let mut grab_meta = HashMap::new();
                                grab_meta.insert(
                                    "title_name".to_string(),
                                    serde_json::json!(title.name),
                                );
                                grab_meta.insert(
                                    "release_title".to_string(),
                                    serde_json::json!(best_pack.title),
                                );
                                grab_meta.insert(
                                    "indexer".to_string(),
                                    serde_json::json!(best_pack.source),
                                );
                                grab_meta
                                    .insert("score".to_string(), serde_json::json!(pack_score));
                                let _ = app
                                    .append_domain_event(new_title_domain_event(
                                        None,
                                        &title,
                                        DomainEventPayload::ReleaseGrabbed(
                                            ReleaseGrabbedEventData {
                                                title: title_context_snapshot(&title),
                                                source_title: Some(best_pack.title.clone()),
                                                source_hint: Some(best_pack.source.clone()),
                                                source_provider: Some(best_pack.source.clone()),
                                                download_id: Some(download_job_id),
                                                episode_ids: grabbed_episode_ids,
                                            },
                                        ),
                                    ))
                                    .await;
                                info!(
                                    title = title.name.as_str(),
                                    season = season_num,
                                    release = best_pack.title.as_str(),
                                    "season pack grabbed; skipping individual episode searches for this season"
                                );
                                break 'season_pack_candidates;
                            }
                            Err(err) => {
                                let submit_unavailable = is_download_submit_unavailable_error(&err);
                                let ambiguous = err.is_download_submit_ambiguous();
                                if submit_unavailable && !failed_routes.contains(&pack_route) {
                                    failed_routes.push(pack_route.clone());
                                    cycle.mark_failed_route(pack_route.clone());
                                }
                                if ambiguous {
                                    cycle.mark_submitted(&url_str);
                                    cycle.mark_season_pack_viable(&season_key);
                                } else if !submit_unavailable {
                                    cycle.clear_season_pack_viable(&season_key);
                                }
                                warn!(
                                    title = title.name.as_str(),
                                    season = season_num,
                                    error = %err,
                                    retry_alternate_route = submit_unavailable,
                                    "season pack grab failed"
                                );
                                // Transient (client unavailable) and ambiguous
                                // (request may have been accepted) submits are
                                // deferred: Pending attempt, never blocklisted.
                                // Only a definitive failure burns the pack.
                                let source_gone = err.is_download_source_gone();
                                let defer = submit_unavailable || ambiguous || source_gone;
                                let _ = app
                                    .services
                                    .workflow
                                    .release_attempts
                                    .record_release_attempt(
                                        Some(title.id.clone()),
                                        pack_hint.clone(),
                                        pack_title_norm.clone(),
                                        if defer {
                                            ReleaseDownloadAttemptOutcome::Pending
                                        } else {
                                            ReleaseDownloadAttemptOutcome::Failed
                                        },
                                        Some(err.to_string()),
                                        pack_password,
                                    )
                                    .await;
                                if !defer && let Some(release_name) = pack_title_norm {
                                    if let Err(error) = app
                                        .services
                                        .workflow
                                        .blocklist_repo
                                        .block(&NewBlocklistEntry {
                                            title_id: title.id.clone(),
                                            release_name,
                                            indexer_id: best_pack
                                                .indexer_id
                                                .clone()
                                                .unwrap_or_default(),
                                            info_hash: best_pack.info_hash().map(str::to_string),
                                            reason: Some(format!("season pack grab failed: {err}")),
                                        })
                                        .await
                                    {
                                        warn!(
                                            error = %error,
                                            title_id = title.id.as_str(),
                                            release = best_pack.title.as_str(),
                                            "failed to persist blocklist entry for failed season pack grab"
                                        );
                                    }
                                }
                                if !submit_unavailable {
                                    break 'season_pack_candidates;
                                }
                            }
                        }
                    }
                }
            }
        }

        // If a season pack was grabbed or remains viable this cycle (by this
        // item or an earlier item for the same season), skip the individual
        // episode search unless the pack submission definitively failed.
        if cycle.season_pack_grabbed(&season_key) {
            return Ok(());
        }
        if cycle.season_pack_viable(&season_key) {
            info!(
                title = title.name.as_str(),
                season = season_num,
                "season pack candidate found; skipping individual episode search for this cycle"
            );
            return Ok(());
        }
    }
    // ── End season pack priority ──────────────────────────────────────────────
    if pack_stage_only {
        return Ok(());
    }
    // Uses the per-facet default download category; the selected client's
    // explicit routing category overrides this inside the router.
    let download_cat = app.derive_download_category(&title.facet).await;

    if subject.queries.is_empty() {
        info!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            media_type = item.media_type.as_str(),
            "background acquisition: no search queries built, skipping"
        );
        return Ok(());
    }

    debug!(
        title_id = title.id.as_str(),
        title_name = title.name.as_str(),
        queries = ?subject.queries,
        imdb_id = subject.imdb_id.as_deref().unwrap_or(""),
        tvdb_id = subject.tvdb_id.as_deref().unwrap_or(""),
        category = subject.category.as_str(),
        "background acquisition: searching indexers"
    );

    let cached_results = search_season
        .map(|season| cycle.season_candidates(&(title.id.clone(), season)))
        .unwrap_or_default()
        .into_iter()
        .filter(|candidate| {
            candidate
                .indexer_id
                .as_ref()
                .is_some_and(|indexer_id| uncovered.contains(indexer_id))
        })
        .collect::<Vec<_>>();
    let cached_results = app
        .evaluate_search_results_for_subject(&search_title, &subject, cached_results, false)
        .await;

    // A complete season query already discovered these episode candidates.
    // Search the individual episode only when that reusable corpus has no
    // eligible result for this scope.
    let results = if cached_results
        .iter()
        .any(|candidate| candidate.auto_eligible == Some(true))
    {
        cached_results
    } else {
        match app
            .search_and_evaluate_subject_restricted(
                &search_title,
                &subject,
                "background_acquisition",
                SearchMode::Auto,
                tokio_util::sync::CancellationToken::new(),
                Some(uncovered),
                Some(if target.is_hot {
                    BACKGROUND_HOT_TARGET_VALUE
                } else {
                    BACKGROUND_COLD_TARGET_VALUE
                }),
            )
            .await
        {
            Ok(r) => r,
            Err(err) => {
                warn!(
                    title_id = title.id.as_str(),
                    error = %err,
                    "background search failed"
                );
                return Ok(());
            }
        }
    };

    // Cooldown state, not cadence: the upgrade policy and failed-grab handling
    // read when this scope last actually searched.
    let _ = app
        .services
        .workflow
        .acquisition_scope_states
        .record_acquisition_scope_search_attempt(&item.id, &now.to_rfc3339())
        .await;

    app.emit_acquisition_search_completed_event(None, &title, results.len() as i64)
        .await;

    if results.is_empty() {
        debug!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            "background acquisition: search returned 0 results"
        );
        return Ok(());
    }

    debug!(
        title_id = title.id.as_str(),
        title_name = title.name.as_str(),
        result_count = results.len(),
        "background acquisition: evaluating candidates"
    );

    // Load the per-title blocklist (covers post-import failures like fake/non-video
    // files, in addition to the download-client snapshot checked below). It is the
    // single, removable exclusion source; the failed-attempt log never gates.
    let db_blocklist = app.load_title_release_blocklist_signatures(&title.id).await;
    let existing_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|file| file.role.is_primary())
        .collect::<Vec<_>>();
    let cutoff_scope = app.cutoff_scope_for(&subject.submission_scope).await;
    let analyzed_cutoff_quality =
        crate::acquisition::decision_helpers::analyzed_cutoff_quality_for_scope(
            &existing_files,
            &cutoff_scope,
        );

    let upgrade_context = match app
        .resolve_upgrade_context_for_title_with_category_and_quality(
            &search_title,
            Some(subject.category.as_str()),
            analyzed_cutoff_quality,
        )
        .await
    {
        Ok(context) => context,
        Err(error) => {
            warn!(
                title_id = title.id.as_str(),
                error = %error,
                "background acquisition: failed to resolve quality profile; skipping target"
            );
            return Ok(());
        }
    };
    let profile = &upgrade_context.profile;

    // The bar the gate compares against, resolved once for the whole loop so the
    // decision log records the file that actually decided each candidate rather
    // than a number remembered on the scope row — and so the cutoff check below
    // can ask about the *score* half of the cutoff, not just the quality half.
    let admission = {
        let scoring_context = app.resolve_canonical_scoring_context(&title, profile).await;
        app.admission_subject_for_scope(
            &title,
            &item.submission_scope(),
            &scoring_context,
            title.runtime_minutes,
            crate::quality::canonical_context::SubjectIntent::Grab,
        )
        .await
    };
    let incumbent_bar = admission.best_score();

    // The one place a scope-level cutoff short-circuit survives (D15).
    //
    // Its siblings in the RSS and pending lanes are gone: the cutoff is now a
    // candidate-aware gate (`cutoff_refusal`) so a PROPER can still reach a
    // scope that has otherwise finished. Active search deliberately does not
    // get that escape — Sonarr's `ProperSpecification` accepts only on the feed
    // lane, so an at-cutoff scope reached by active search stops here.
    //
    // **Both halves of the cutoff**, as Sonarr reads it: the quality has
    // arrived *and* the bar has reached `cutoff_score`. Gating on the quality
    // alone abandoned every target `derive_format_cutoff_targets` produces —
    // quality at cutoff, score below it — which is exactly the population D19
    // exists to re-search.
    //
    // It is *not* a pre-search return, despite where it sits: the indexer query
    // already ran above. What it saves is the candidate loop — one decision row
    // per result plus the grab attempts.
    if crate::acquisition_release_search::incumbent_at_cutoff(
        upgrade_context.cutoff_reached,
        &admission,
        profile.criteria.cutoff_score,
    ) {
        tracing::debug!(
            title_id = title.id.as_str(),
            cutoff = profile.criteria.cutoff_tier.as_deref().unwrap_or(""),
            cutoff_score = profile.criteria.cutoff_score,
            "cutoff reached, skipping upgrade"
        );
        return Ok(());
    }
    let delay_profiles = app.load_delay_profiles().await;

    // ── Candidate fallthrough loop ──────────────────────────────────────────
    // Iterate ranked candidates (sorted by preference_score DESC).  If a grab
    // fails, try the next candidate instead of re-searching from scratch next
    // cycle.  Mirrors Sonarr's ProcessDownloadDecisions loop.
    let mut had_allowed_candidate = false;
    let mut had_quality_allowed_candidate = false;
    let mut skipped_for_failed = false;
    let mut skipped_for_title_mismatch = false;
    // Park the best ambiguous candidate before a higher-ranked eligible release
    // can return from the loop. Otherwise the pending-review side effect depends
    // on incidental candidate ordering.
    let mut parked_ambiguous_identity = false;
    if let Some(candidate) = results.iter().find(|candidate| {
        candidate
            .quality_profile_decision
            .as_ref()
            .is_some_and(|decision| decision.allowed)
            && matches!(
                effective_auto_decision_code_for_route(candidate, &failed_routes, &db_blocklist),
                ReleaseAutoDecisionCode::AmbiguousIdentity
            )
    }) {
        parked_ambiguous_identity = true;
        let candidate_score = candidate
            .quality_profile_decision
            .as_ref()
            .map(|decision| decision.preference_score)
            .unwrap_or_default();
        app.park_pending_release_for_review(
            item,
            &title,
            candidate,
            candidate_score,
            serialize_decision_explanation(candidate),
        )
        .await;
    }
    let mut grab_attempts: usize = 0;
    let mut next_pending_role = PendingReleaseRole::Primary;

    for (candidate_index, candidate) in results.iter().enumerate() {
        let is_allowed = candidate
            .quality_profile_decision
            .as_ref()
            .map(|d| d.allowed)
            .unwrap_or(false);
        let decision_code = if is_allowed {
            effective_auto_decision_code_for_route(candidate, &failed_routes, &db_blocklist)
        } else {
            ReleaseAutoDecisionCode::QualityBlocked
        };
        if !is_allowed {
            // Blocked on quality alone: admission never looked at an incumbent.
            record_release_decision(app, item, &title, candidate, decision_code, None, now).await;
            app.emit_acquisition_candidate_rejected_event(
                None,
                &title,
                candidate.title.clone(),
                decision_code.as_str().to_string(),
            )
            .await;
            continue;
        }

        had_quality_allowed_candidate = true;

        let candidate_score = candidate
            .quality_profile_decision
            .as_ref()
            .map(|d| d.preference_score)
            .unwrap_or(0);

        if !matches!(
            decision_code,
            ReleaseAutoDecisionCode::TitleMismatch
                | ReleaseAutoDecisionCode::EpisodeMismatch
                | ReleaseAutoDecisionCode::CategoryMismatch
                | ReleaseAutoDecisionCode::AmbiguousIdentity
        ) {
            had_allowed_candidate = true;
        }
        if matches!(
            decision_code,
            ReleaseAutoDecisionCode::TitleMismatch
                | ReleaseAutoDecisionCode::EpisodeMismatch
                | ReleaseAutoDecisionCode::CategoryMismatch
                | ReleaseAutoDecisionCode::AmbiguousIdentity
        ) {
            skipped_for_title_mismatch = true;
        }
        if matches!(decision_code, ReleaseAutoDecisionCode::DbBlocklisted) {
            skipped_for_failed = true;
        }

        record_release_decision(
            app,
            item,
            &title,
            candidate,
            decision_code,
            incumbent_bar,
            now,
        )
        .await;

        if !decision_code.is_eligible() {
            app.emit_acquisition_candidate_rejected_event(
                None,
                &title,
                candidate.title.clone(),
                decision_code.as_str().to_string(),
            )
            .await;
            // A fact about the *scope*, not about this candidate: the ranked
            // order is (tier, revision, score) and admission compares the same
            // three in the same order, so nothing below a rejected candidate
            // can do better either.
            //
            // `CutoffReached` used to be listed here and no longer is. Since
            // D15 it is candidate-aware — a same-tier revision upgrade escapes
            // it — and a better-*tier* candidate refused by the cutoff sorts
            // *above* the same-tier PROPER that would pass, so breaking on it
            // would skip the one release worth having. (`NegativeScore` was
            // also listed once; nothing emits it any more — the hardcoded zero
            // floor is gone — and the variant survives only so historical
            // decision rows still decode.)
            if matches!(decision_code, ReleaseAutoDecisionCode::UpgradeRejected) {
                break;
            }
            if matches!(decision_code, ReleaseAutoDecisionCode::AmbiguousIdentity)
                && !parked_ambiguous_identity
            {
                parked_ambiguous_identity = true;
                app.park_pending_release_for_review(
                    item,
                    &title,
                    candidate,
                    candidate_score,
                    serialize_decision_explanation(candidate),
                )
                .await;
                // Keep walking the ranked list: a lower-scored candidate that
                // does present a disambiguator is still grabbable this cycle.
                continue;
            }
            if matches!(
                decision_code,
                ReleaseAutoDecisionCode::PendingDelay
                    | ReleaseAutoDecisionCode::MinimumAge
                    | ReleaseAutoDecisionCode::ReleaseAgeUnknown
            ) {
                let scoring_json = candidate.quality_profile_decision.as_ref().map(|decision| {
                    serde_json::to_string(
                        &decision
                            .scoring_log
                            .iter()
                            .map(|entry| serde_json::json!({"code": entry.code, "delta": entry.delta}))
                            .collect::<Vec<_>>(),
                    )
                    .unwrap_or_default()
                });

                let canonical_source = candidate.canonical_download_source();
                let parsed_published_at = candidate
                    .published_at
                    .as_deref()
                    .and_then(crate::quality_profile::parse_published_at);
                let normalized_published_at =
                    parsed_published_at.map(|published_at| published_at.to_rfc3339());
                let delay = automatic_candidate_delay_decision(
                    candidate,
                    &search_title,
                    &admission,
                    profile,
                    &delay_profiles,
                    false,
                    None,
                    now,
                );
                let eligible_at =
                    if matches!(decision_code, ReleaseAutoDecisionCode::ReleaseAgeUnknown) {
                        crate::delay_profile::resolve_delay_profile(
                            &delay_profiles,
                            &search_title.tags,
                            &search_title.facet,
                        )
                        .map(|profile| {
                            profile.release_age_unknown_escalation_deadline(
                                candidate.source_kind,
                                *now,
                            )
                        })
                        .unwrap_or(*now)
                    } else {
                        delay
                            .and_then(|decision| decision.eligible_at)
                            .unwrap_or(*now)
                    };
                let pending = PendingRelease {
                    id: Id::new().0,
                    wanted_item_id: item.id.clone(),
                    title_id: title.id.clone(),
                    release_title: candidate.title.clone(),
                    release_url: canonical_source.as_ref().map(|(source, _)| source.clone()),
                    source_kind: canonical_source
                        .as_ref()
                        .map(|(_, kind)| *kind)
                        .or(candidate.source_kind),
                    release_size_bytes: candidate.size_bytes,
                    release_score: candidate_score,
                    scoring_log_json: scoring_json,
                    indexer_source: Some(candidate.source.clone()),
                    indexer_id: candidate.indexer_id.clone(),
                    release_guid: candidate.guid.clone(),
                    added_at: now.to_rfc3339(),
                    last_observed_at: now.to_rfc3339(),
                    delay_until: eligible_at.to_rfc3339(),
                    status: PendingReleaseStatus::Waiting,
                    grabbed_at: None,
                    source_password: crate::normalize_release_password(
                        candidate.password_hint.as_deref(),
                    ),
                    published_at: normalized_published_at,
                    info_hash: candidate
                        .extra
                        .get("info_hash")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                    seed_minimums: crate::ReleaseSeedMinimums::from_release_extra(&candidate.extra),
                    seeders: crate::acquisition::seed_goals::seeders_from_extra(&candidate.extra),
                    release_identity: String::new(),
                    coverage_identity: String::new(),
                    role: next_pending_role,
                    last_decision_code: Some(decision_code.as_str().to_string()),
                    release_age_unknown: matches!(
                        decision_code,
                        ReleaseAutoDecisionCode::ReleaseAgeUnknown
                    ),
                };
                let observation = PendingReleaseObservation::derived(&pending, next_pending_role);
                match app
                    .insert_pending_release_observation(&pending, &observation)
                    .await
                {
                    Ok(_) => next_pending_role = PendingReleaseRole::Fallback,
                    Err(error) => {
                        warn!(
                            error = %error,
                            title = title.name.as_str(),
                            release = candidate.title.as_str(),
                            decision = decision_code.as_str(),
                            "pending release: failed to persist automatic search hold"
                        );
                    }
                }
            }
            continue;
        }

        // ── Grab attempt ────────────────────────────────────────────────────
        grab_attempts += 1;
        if grab_attempts > 10 {
            warn!(
                title = title.name.as_str(),
                "reached max grab attempts (10), deferring to next cycle"
            );
            break;
        }

        // Submit to download client
        let canonical_source = candidate.canonical_download_source();
        let source_hint = canonical_source.as_ref().map(|(source, _)| source.clone());

        // Successful or ambiguous submissions stay globally deduplicated, but
        // a failed URL is suppressed only within its source/indexer route.
        if let Some(url) = source_hint.as_deref() {
            let route = DownloadRouteKey::for_candidate(candidate)
                .expect("candidate route key always exists, including unknown source kind");
            match cycle.claim_submission(route, url) {
                SubmissionClaim::Granted => {}
                SubmissionClaim::AlreadySubmitted => {
                    info!(
                        title = title.name.as_str(),
                        release = candidate.title.as_str(),
                        "skipping duplicate release already submitted this cycle"
                    );
                    continue;
                }
                SubmissionClaim::AlreadyAttempted | SubmissionClaim::RouteUnavailable => {
                    info!(
                        title = title.name.as_str(),
                        release = candidate.title.as_str(),
                        indexer_id = ?candidate.indexer_id,
                        source_kind = ?candidate.source_kind,
                        "skipping duplicate release already attempted or unavailable this cycle"
                    );
                    continue;
                }
            }
        }

        let source_title = Some(candidate.title.clone());
        let canonical_source_kind = canonical_source
            .as_ref()
            .map(|(_, kind)| *kind)
            .or(candidate.source_kind);
        let source_hint_for_attempt = normalize_release_attempt_hint(source_hint.as_deref());
        let source_title_for_attempt = normalize_release_name(source_title.as_deref());
        let source_password = normalize_release_password(candidate.password_hint.as_deref());
        let request_signature = normalize_release_selection_signature(
            source_hint.as_deref(),
            source_title.as_deref(),
            canonical_source_kind,
        );

        let _ = app
            .services
            .workflow
            .release_attempts
            .record_release_attempt(
                Some(title.id.clone()),
                source_hint_for_attempt.clone(),
                source_title_for_attempt.clone(),
                ReleaseDownloadAttemptOutcome::Pending,
                None,
                source_password.clone(),
            )
            .await;

        let is_recent = app.is_recent_for_queue_priority(
            candidate
                .published_at
                .as_deref()
                .or(episode.as_ref().and_then(|item| item.air_date.as_deref()))
                .or(title.first_aired.as_deref())
                .or(title.digital_release_date.as_deref()),
        );

        info!(
            title = title.name.as_str(),
            release = candidate.title.as_str(),
            score = candidate_score,
            decision = decision_code.as_str(),
            attempt = grab_attempts,
            "auto-grabbing release"
        );

        let info_hash_hint = candidate.info_hash().map(str::to_string);
        let seed_minimums = crate::ReleaseSeedMinimums::from_release_extra(&candidate.extra);
        // This path used to hardcode `season_pack: false`; the scored candidate
        // already carries a parse, so the seeding resolver can see a real pack.
        let is_season_pack = candidate
            .parsed_release_metadata
            .as_ref()
            .and_then(|parsed| parsed.episode.as_ref())
            .is_some_and(|episode| episode.full_season);
        let download_id = scryer_domain::download_identity::DownloadId::new();
        let submission_scope = if let Some(parsed) = candidate.parsed_release_metadata.as_ref() {
            let catalog_episodes = app
                .services
                .catalog
                .shows
                .list_episodes_for_title(&title.id)
                .await
                .unwrap_or_default();
            let catalog_collections = app
                .services
                .catalog
                .shows
                .list_collections_for_title(&title.id)
                .await
                .unwrap_or_default();
            crate::acquisition_coverage::resolve_release_coverage(
                parsed,
                &catalog_episodes,
                &catalog_collections,
                episode.as_ref(),
            )
            .submission_scope_or(&direct_download_submission_scope_for_wanted_item(
                item,
                episode.as_ref(),
            ))
        } else {
            direct_download_submission_scope_for_wanted_item(item, episode.as_ref())
        };

        let canonical_result = app
            .submit_canonical_download(CanonicalDownloadSubmissionIntent {
                request: DownloadClientAddRequest {
                    title: title.clone(),
                    search_facet: (target.media_type == "series_movie")
                        .then_some(MediaFacet::Movie),
                    purpose: crate::DownloadSubmissionPurpose::Standard,
                    download_id: Some(download_id),
                    source_hint: source_hint.clone(),
                    staged_nzb: None,
                    resolved_download_artifact: None,
                    source_kind: canonical_source_kind,
                    source_title: source_title.clone(),
                    source_password: source_password.clone(),
                    category: Some(download_cat.clone()),
                    queue_priority: None,
                    download_directory: None,
                    release_title: Some(candidate.title.clone()),
                    indexer_name: Some(candidate.source.clone()),
                    indexer_id: candidate.indexer_id.clone(),
                    info_hash_hint: info_hash_hint.clone(),
                    seed_goal_ratio: None,
                    seed_goal_seconds: None,
                    tracker_min_seed_ratio: seed_minimums.min_seed_ratio,
                    tracker_min_seed_time_minutes: seed_minimums.min_seed_time_minutes,
                    season_pack_seed_ratio: seed_minimums.season_pack_seed_ratio,
                    season_pack_seed_time_minutes: seed_minimums.season_pack_seed_time_minutes,
                    is_recent,
                    season_pack: Some(is_season_pack),
                },
                scope: submission_scope.clone(),
                conflict_policy: SubmissionConflictPolicy::Skip,
                request_signature: request_signature.clone(),
                source_provider_name: Some(candidate.source.clone()),
                release_size_bytes: candidate.size_bytes,
            })
            .await;

        let canonical_submission = match canonical_result {
            Ok(CanonicalDownloadSubmissionOutcome::Accepted(submission)) => Ok(submission),
            Ok(CanonicalDownloadSubmissionOutcome::Conflict(_)) => return Ok(()),
            Err(error) => Err(error),
        };

        match canonical_submission {
            Ok(canonical_submission) => {
                let grab = canonical_submission.grab;
                // ── Success ─────────────────────────────────────────────────
                if let Some(url) = source_hint.as_deref() {
                    cycle.mark_submitted(url);
                }
                {
                    let facet_label = serde_json::to_string(&title.facet)
                        .unwrap_or_else(|_| "\"other\"".to_string())
                        .trim_matches('"')
                        .to_string();
                    metrics::counter!("scryer_grabs_total", "indexer" => candidate.source.clone(), "facet" => facet_label).increment(1);
                }
                app.record_indexer_grab(
                    candidate.indexer_id.as_deref(),
                    Some(candidate.source.as_str()),
                );
                let _ = app
                    .services
                    .workflow
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt.clone(),
                        source_title_for_attempt.clone(),
                        ReleaseDownloadAttemptOutcome::Success,
                        None,
                        source_password.clone(),
                    )
                    .await;

                // Record title history: Grabbed
                // Record download submission for auto-import matching
                let grabbed_json = serde_json::json!({
                    "title": candidate.title,
                    "score": candidate_score,
                    "grabbed_at": now.to_rfc3339(),
                    "source_provider": candidate.source.clone(),
                })
                .to_string();
                let download_job_id = grab.job_id.clone();
                let covered_wanted_item_ids = app
                    .covered_wanted_item_ids_for_submission_scope(
                        &title.id,
                        &submission_scope,
                        &item.id,
                    )
                    .await?;

                app.services
                    .workflow
                    .acquisition_state
                    .commit_successful_grab(&SuccessfulGrabCommit {
                        wanted_item_id: item.id.clone(),
                        covered_wanted_item_ids,
                        grabbed_release: grabbed_json,
                        last_search_at: Some(now.to_rfc3339()),
                        grabbed_pending_release_id: None,
                        grabbed_at: Some(now.to_rfc3339()),
                    })
                    .await?;
                persist_standby_candidates(
                    app,
                    item,
                    &title,
                    &results,
                    candidate_index + 1,
                    now,
                    &failed_routes,
                    &db_blocklist,
                    |_| true,
                )
                .await;

                let _ = app
                    .append_domain_event(new_title_domain_event(
                        None,
                        &title,
                        DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                            title: title_context_snapshot(&title),
                            source_title: Some(candidate.title.clone()),
                            source_hint: Some(candidate.source.clone()),
                            source_provider: Some(candidate.source.clone()),
                            download_id: Some(download_job_id),
                            episode_ids: item.episode_id.iter().cloned().collect(),
                        }),
                    ))
                    .await;

                return Ok(());
            }
            Err(err) => {
                if err.is_download_submit_ambiguous() {
                    if let Some(url) = source_hint.as_deref() {
                        cycle.mark_submitted(url);
                    }
                    warn!(
                        title = title.name.as_str(),
                        release = candidate.title.as_str(),
                        attempt = grab_attempts,
                        error = %err,
                        "download submission result is ambiguous; re-opening scope without blocklisting or failover"
                    );

                    if let Some(scope_key) = convergence_scope_key.as_deref() {
                        app.prune_scope_key_coverage(scope_key, candidate.indexer_id.as_deref())
                            .await;
                    }

                    return Ok(());
                }

                // ── Grab failed — try next candidate ────────────────────────
                warn!(
                    title = title.name.as_str(),
                    release = candidate.title.as_str(),
                    attempt = grab_attempts,
                    error = %err,
                    "grab failed, trying next candidate"
                );

                let failure_reason = format!(
                    "grab failed for '{}' (attempt {}/10, trying next): {}",
                    candidate.title, grab_attempts, err
                );
                let source_gone = err.is_download_source_gone();
                let submit_unavailable = is_download_submit_unavailable_error(&err) || source_gone;

                if source_gone {
                    info!(
                        release = candidate.title.as_str(),
                        "download source gone; leaving it unblocked outside standby recovery"
                    );
                }

                if submit_unavailable {
                    let _ = app
                        .services
                        .workflow
                        .release_attempts
                        .record_release_attempt(
                            Some(title.id.clone()),
                            source_hint_for_attempt.clone(),
                            source_title_for_attempt.clone(),
                            ReleaseDownloadAttemptOutcome::Pending,
                            Some(failure_reason.clone()),
                            source_password.clone(),
                        )
                        .await;
                } else {
                    let attribution = FailedReleaseAttribution {
                        title: Some(title.clone()),
                        episode_ids: item.episode_id.iter().cloned().collect(),
                        collection_id: item.collection_id.clone(),
                    };
                    let candidate_source_hint = candidate
                        .canonical_download_source()
                        .map(|(source, _)| source)
                        .unwrap_or_else(|| candidate.source.clone());
                    let quality = candidate
                        .parsed_release_metadata
                        .as_ref()
                        .and_then(|parsed| parsed.quality.clone())
                        .or_else(|| release_quality_hint(Some(candidate.title.as_str())));

                    // A definitive grab failure burns the release for this title:
                    // the per-title blocklist entry is what search-time exclusion
                    // consults (and what the operator can remove); the Failed
                    // attempt is the audit record. Transient failures never
                    // reach here (Pending above).
                    record_failed_release_outcome(
                        app,
                        Some(title.id.as_str()),
                        &attribution,
                        Some(candidate.title.clone()),
                        Some(candidate_source_hint),
                        candidate.indexer_id.clone().unwrap_or_default(),
                        candidate.info_hash().map(str::to_string),
                        None,
                        None,
                        None,
                        None,
                        quality,
                        Some(failure_reason),
                        Some(format!("grab failed: {err}")),
                        source_password.clone(),
                    )
                    .await;
                }

                // If download-client submit is unavailable, suppress only this
                // source/indexer route for the remainder of this cycle.
                if submit_unavailable
                    && let Some(route) = DownloadRouteKey::for_candidate(candidate)
                {
                    if !failed_routes.contains(&route) {
                        failed_routes.push(route.clone());
                        cycle.mark_failed_route(route.clone());
                    }
                    info!(
                        source_kind = ?route.source_kind,
                        indexer_id = ?route.indexer_id,
                        "download client submit unavailable for route, skipping remaining candidates on this route"
                    );
                }

                // CONTINUE — try the next candidate
            }
        }
    }
    // ── End candidate fallthrough loop ───────────────────────────────────────

    // All candidates exhausted without a successful grab.
    if grab_attempts > 0 {
        warn!(
            title = title.name.as_str(),
            attempts = grab_attempts,
            "all grab attempts failed, re-queuing for next cycle"
        );
    } else if had_allowed_candidate && skipped_for_failed {
        warn!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            "background acquisition: no suitable candidates found after skipping blocklisted or active releases"
        );
    } else if had_allowed_candidate {
        debug!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            "background acquisition: all allowed candidates were already active or had negative scores"
        );
    } else if had_quality_allowed_candidate && skipped_for_title_mismatch {
        debug!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            result_count = results.len(),
            "background acquisition: quality-allowed candidates were rejected by title matching"
        );
    } else {
        debug!(
            title_id = title.id.as_str(),
            title_name = title.name.as_str(),
            result_count = results.len(),
            "background acquisition: no allowed candidates found (all blocked by quality profile)"
        );
    }

    // No grab this cycle: the scope's coverage now reflects every indexer that
    // answered, so the cursor will not re-search them — new postings arrive via
    // RSS, and any still-uncovered indexers are retried on a later rotation.
    Ok(())
}

pub async fn start_background_acquisition_poller(
    app: AppUseCase,
    token: tokio_util::sync::CancellationToken,
) {
    // Check feature flag
    let enabled = std::env::var("SCRYER_BACKGROUND_ACQUISITION")
        .map(|v| !matches!(v.to_lowercase().as_str(), "false" | "0" | "no" | "off"))
        .unwrap_or(true);

    if !enabled {
        info!("background acquisition poller is disabled (SCRYER_BACKGROUND_ACQUISITION=false)");
        return;
    }

    let settings = match app.acquisition_settings().await {
        Ok(settings) => settings,
        Err(err) => {
            warn!(error = %err, "failed to load acquisition settings, using defaults");
            crate::AcquisitionSettings {
                enabled: true,
                upgrade_cooldown_hours: 24,
                same_tier_min_delta: 120,
                cross_tier_min_delta: 30,
                forced_upgrade_delta_bypass: 400,
                poll_interval_seconds: 60,
                long_tail_backfill_max_scopes_per_cycle:
                    crate::acquisition::convergence::DEFAULT_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE
                        as i32,
                long_tail_reconverge_days: 0,
            }
        }
    };

    if !settings.enabled {
        info!("background acquisition poller is disabled (acquisition.enabled != true)");
        return;
    }

    info!("background acquisition poller started");

    // Run-once cutover seed: recently-searched legacy scopes
    // start converged so first boot does not re-sweep the back-catalog.
    // Spawned so startup stays non-blocking; the cycle racing the seed is
    // harmless (either path only causes a safe converge).
    {
        let app = app.clone();
        tokio::spawn(async move {
            app.seed_convergence_from_legacy_history().await;
        });
    }

    // Run initial health checks after a short delay to let services initialize
    {
        let app = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            if let Err(error) = app
                .run_scheduled_job_now(JobKey::HealthChecks, JobTriggerSource::ScheduledStartup)
                .await
            {
                warn!(error = %error, "initial health checks failed");
            }
        });
    }

    // Refresh managed Prowlarr children as soon as the app is up so startup
    // picks up upstream indexer/config changes without waiting for the first
    // 5-minute interval.
    {
        let app = app.clone();
        tokio::spawn(async move {
            if let Err(error) = app
                .run_scheduled_job_now(JobKey::ProwlarrSync, JobTriggerSource::ScheduledStartup)
                .await
            {
                warn!(error = %error, "initial Prowlarr sync failed");
            }
        });
    }
    {
        let app = app.clone();
        tokio::spawn(async move {
            let actor = scryer_domain::User::new_admin("system-indexer-caps");
            if let Err(error) = app.refresh_enabled_direct_nab_caps_snapshots(&actor).await {
                warn!(error = %error, "initial direct indexer caps refresh failed");
            }
        });
    }

    app.set_job_next_run_at(
        JobKey::PluginRegistryRefresh,
        Utc::now() + chrono::Duration::hours(1),
    )
    .await;
    app.set_job_next_run_at(
        JobKey::HealthChecks,
        Utc::now() + chrono::Duration::seconds(30),
    )
    .await;
    app.set_job_next_run_at(
        JobKey::StagedNzbPrune,
        Utc::now() + chrono::Duration::hours(1),
    )
    .await;
    app.set_job_next_run_at(
        JobKey::Housekeeping,
        Utc::now() + chrono::Duration::hours(24),
    )
    .await;
    app.set_job_next_run_at(
        JobKey::ProwlarrSync,
        Utc::now() + chrono::Duration::minutes(5),
    )
    .await;
    app.set_job_next_run_at(JobKey::RssSync, Utc::now() + chrono::Duration::minutes(1))
        .await;
    app.set_job_next_run_at(
        JobKey::PendingReleaseProcessing,
        Utc::now() + chrono::Duration::minutes(1),
    )
    .await;

    let mut poll_interval = new_skip_interval(std::time::Duration::from_secs(
        settings.poll_interval_seconds.max(1) as u64,
    ));
    let mut registry_refresh_interval = tokio::time::interval(std::time::Duration::from_hours(1));
    let mut health_check_interval = tokio::time::interval(std::time::Duration::from_hours(6));
    let mut staged_nzb_prune_interval = tokio::time::interval(std::time::Duration::from_hours(1));
    let mut housekeeping_interval = tokio::time::interval(std::time::Duration::from_hours(24));
    let mut prowlarr_sync_interval = tokio::time::interval(std::time::Duration::from_mins(5));
    let mut direct_indexer_caps_interval =
        tokio::time::interval(std::time::Duration::from_hours(24));
    let mut rss_sync_interval = tokio::time::interval(std::time::Duration::from_mins(1));
    let mut pending_release_interval = tokio::time::interval(std::time::Duration::from_mins(1));

    // Consume immediate intervals.
    poll_interval.tick().await;
    registry_refresh_interval.tick().await;
    health_check_interval.tick().await;
    staged_nzb_prune_interval.tick().await;
    housekeeping_interval.tick().await;
    prowlarr_sync_interval.tick().await;
    direct_indexer_caps_interval.tick().await;
    rss_sync_interval.tick().await;
    pending_release_interval.tick().await;

    {
        let app = app.clone();
        let token = token.child_token();
        tokio::spawn(async move {
            run_discovery_sync_worker(app, token).await;
        });
    }

    let wake = app.runtime.acquisition.acquisition_wake.clone();

    /// Run a scheduled task inside a spawned task to isolate panics.
    /// If the task panics, the error is logged and the scheduler loop continues.
    async fn run_task(
        task_name: &'static str,
        fut: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        let t = std::time::Instant::now();
        match tokio::spawn(fut).await {
            Ok(()) => {}
            Err(e) => {
                tracing::error!(
                    task = task_name,
                    error = %e,
                    "CRITICAL: scheduled task panicked — scheduler continues but this task failed"
                );
                metrics::counter!("scryer_task_panics_total", "task" => task_name).increment(1);
            }
        }
        metrics::counter!("scryer_task_runs_total", "task" => task_name).increment(1);
        metrics::histogram!("scryer_task_duration_seconds", "task" => task_name)
            .record(t.elapsed().as_secs_f64());
    }

    loop {
        tokio::select! {
            _ = token.cancelled() => {
                info!("background acquisition poller shutting down");
                break;
            }
            _ = wake.notified() => {
                let app = app.clone();
                run_task("background_acquisition_cycle", async move {
                    run_background_acquisition_cycle(&app).await;
                }).await;
            }
            _ = poll_interval.tick() => {
                let app = app.clone();
                run_task("background_acquisition_cycle", async move {
                    run_background_acquisition_cycle(&app).await;
                }).await;
            }
            _ = registry_refresh_interval.tick() => {
                let app = app.clone();
                run_task("registry_refresh", async move {
                    app.set_job_next_run_at(
                        JobKey::PluginRegistryRefresh,
                        Utc::now() + chrono::Duration::hours(1),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::PluginRegistryRefresh, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic plugin registry refresh failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "registry_refresh").increment(1);
                    }
                }).await;
            }
            _ = health_check_interval.tick() => {
                let app = app.clone();
                run_task("health_check", async move {
                    app.set_job_next_run_at(
                        JobKey::HealthChecks,
                        Utc::now() + chrono::Duration::hours(6),
                    ).await;
                    if let Err(err) = app.run_scheduled_job_now(JobKey::HealthChecks, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %err, "periodic health checks failed");
                    }
                }).await;
            }
            _ = staged_nzb_prune_interval.tick() => {
                let app = app.clone();
                run_task("staged_nzb_prune", async move {
                    app.set_job_next_run_at(
                        JobKey::StagedNzbPrune,
                        Utc::now() + chrono::Duration::hours(1),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::StagedNzbPrune, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic staged nzb prune failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "staged_nzb_prune").increment(1);
                    }
                }).await;
            }
            _ = housekeeping_interval.tick() => {
                let app = app.clone();
                run_task("housekeeping", async move {
                    app.set_job_next_run_at(
                        JobKey::Housekeeping,
                        Utc::now() + chrono::Duration::hours(24),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::Housekeeping, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic housekeeping failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "housekeeping").increment(1);
                    }
                }).await;
            }
            _ = pending_release_interval.tick() => {
                let app = app.clone();
                run_task("pending_releases", async move {
                    app.set_job_next_run_at(
                        JobKey::PendingReleaseProcessing,
                        Utc::now() + chrono::Duration::minutes(1),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::PendingReleaseProcessing, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "pending release processor failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "pending_releases").increment(1);
                    }
                }).await;
            }
            _ = prowlarr_sync_interval.tick() => {
                let app = app.clone();
                run_task("prowlarr_sync", async move {
                    app.set_job_next_run_at(
                        JobKey::ProwlarrSync,
                        Utc::now() + chrono::Duration::minutes(5),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::ProwlarrSync, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic Prowlarr sync failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "prowlarr_sync").increment(1);
                    }
                }).await;
            }
            _ = direct_indexer_caps_interval.tick() => {
                let app = app.clone();
                run_task("direct_indexer_caps", async move {
                    let actor = scryer_domain::User::new_admin("system-indexer-caps");
                    if let Err(error) = app.refresh_enabled_direct_nab_caps_snapshots(&actor).await {
                        warn!(error = %error, "periodic direct indexer caps refresh failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "direct_indexer_caps").increment(1);
                    }
                }).await;
            }
            _ = rss_sync_interval.tick() => {
                let app = app.clone();
                run_task("rss_sync", async move {
                    app.set_job_next_run_at(
                        JobKey::RssSync,
                        Utc::now() + chrono::Duration::minutes(1),
                    ).await;
                    if let Err(e) = app.run_scheduled_job_now(JobKey::RssSync, JobTriggerSource::ScheduledInterval).await {
                        warn!(error = %e, "periodic RSS sync failed");
                        metrics::counter!("scryer_task_errors_total", "task" => "rss_sync").increment(1);
                    }
                }).await;
            }
        }
    }
}

async fn run_discovery_sync_worker(app: AppUseCase, token: tokio_util::sync::CancellationToken) {
    // The acquisition poller spawns this worker, so awaiting the startup pass here
    // keeps service startup nonblocking while preventing overlapping discovery runs.
    let discovery_sync_wake = app.runtime.jobs.discovery_sync_wake.clone();
    let mut delay = tokio::select! {
        _ = token.cancelled() => return,
        delay = run_discovery_sync_once(&app, JobTriggerSource::ScheduledStartup) => delay,
    };

    loop {
        tokio::select! {
            _ = token.cancelled() => return,
            _ = tokio::time::sleep(delay) => {}
            _ = discovery_sync_wake.notified() => {}
        }

        if let Some(next_run_at) = app
            .runtime
            .jobs
            .job_run_tracker
            .next_run_at(JobKey::DiscoverySync)
            .await
            && next_run_at > Utc::now()
        {
            delay = discovery_sync_delay_until(next_run_at);
            continue;
        }

        delay = run_discovery_sync_once(&app, JobTriggerSource::ScheduledInterval).await;
    }
}

async fn run_discovery_sync_once(
    app: &AppUseCase,
    trigger_source: JobTriggerSource,
) -> std::time::Duration {
    let started = std::time::Instant::now();
    if let Err(error) = app
        .run_scheduled_job_now(JobKey::DiscoverySync, trigger_source)
        .await
    {
        warn!(
            error = %error,
            trigger_source = trigger_source.as_str(),
            "discovery sync failed"
        );
        metrics::counter!("scryer_task_errors_total", "task" => "discovery_sync").increment(1);
    }
    metrics::counter!("scryer_task_runs_total", "task" => "discovery_sync").increment(1);
    metrics::histogram!("scryer_task_duration_seconds", "task" => "discovery_sync")
        .record(started.elapsed().as_secs_f64());

    app.runtime
        .jobs
        .job_run_tracker
        .next_run_at(JobKey::DiscoverySync)
        .await
        .map(discovery_sync_delay_until)
        .unwrap_or_else(|| std::time::Duration::from_secs(24 * 60 * 60))
}

fn discovery_sync_delay_until(next_run_at: DateTime<Utc>) -> std::time::Duration {
    (next_run_at - Utc::now())
        .to_std()
        .ok()
        .filter(|delay| *delay >= std::time::Duration::from_secs(60))
        .unwrap_or_else(|| std::time::Duration::from_secs(60))
}

#[cfg(test)]
mod task_runner_tests {
    use super::*;
    use crate::acquisition::targets::AcquisitionTarget;

    #[test]
    fn non_metadata_scheduled_job_intervals_remain_unchanged() {
        assert_eq!(JobKey::RssSync.interval_seconds(), Some(15 * 60));
        assert_eq!(
            JobKey::PluginRegistryRefresh.interval_seconds(),
            Some(60 * 60)
        );
        assert_eq!(JobKey::HealthChecks.interval_seconds(), Some(6 * 60 * 60));
        assert_eq!(JobKey::StagedNzbPrune.interval_seconds(), Some(60 * 60));
    }

    #[test]
    fn discovery_sync_delay_until_clamps_stale_times() {
        let stale = Utc::now() - chrono::Duration::minutes(5);
        assert_eq!(
            discovery_sync_delay_until(stale),
            std::time::Duration::from_secs(60)
        );
    }

    fn wanted_episode_item(
        title_id: &str,
        title_name: &str,
        episode_number: u32,
    ) -> AcquisitionScopeState {
        AcquisitionScopeState {
            id: format!("{title_id}-e{episode_number}"),
            title_id: title_id.to_string(),
            title_name: Some(title_name.to_string()),
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: Some(format!("{title_id}-episode-{episode_number}")),
            collection_id: None,
            series_movie_link_id: None,
            season_number: Some("1".to_string()),
            episode_number: Some(episode_number.to_string()),
            media_type: "episode".to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    fn episode_submission(title_id: &str, episode_id: &str, job_id: &str) -> DownloadSubmission {
        DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title_id.to_string(),
            purpose: DownloadSubmissionPurpose::Standard,
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: job_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Bluey.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Episode {
                episode_id: episode_id.to_string(),
            },
        }
    }

    fn snapshot_with_job(job_id: &str, completed: bool) -> DownloadClientSnapshot {
        let key = download_client_item_identity(Some("primary"), job_id);
        let mut snapshot = DownloadClientSnapshot {
            active_titles: Default::default(),
            active_client_ids: Default::default(),
            active_raw_item_id_counts: Default::default(),
            completed_client_ids: Default::default(),
            completed_raw_item_id_counts: Default::default(),
            failed_by_download_id: Default::default(),
            queue_listing_failed: false,
            history_listing_failed: false,
        };
        if completed {
            snapshot.completed_client_ids.insert(key);
        } else {
            snapshot.active_client_ids.insert(key);
        }
        snapshot
    }

    /// The queue could not be listed at all this cycle.
    fn blind_queue_snapshot() -> DownloadClientSnapshot {
        DownloadClientSnapshot {
            active_titles: Default::default(),
            active_client_ids: Default::default(),
            active_raw_item_id_counts: Default::default(),
            completed_client_ids: Default::default(),
            completed_raw_item_id_counts: Default::default(),
            failed_by_download_id: Default::default(),
            queue_listing_failed: true,
            history_listing_failed: false,
        }
    }

    #[test]
    fn completed_submission_blocks_initial_wanted_search() {
        let item = wanted_episode_item("title-bluey", "Bluey", 1);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-baseline");
        let snapshot = snapshot_with_job("job-baseline", true);

        // Nothing occupies the scope yet, so the finished download is still on
        // its way to becoming a file: searching again would duplicate it.
        assert!(submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
            None,
            false,
        ));
    }

    #[test]
    fn failed_submission_does_not_block_completed_initial_wanted_search() {
        let item = wanted_episode_item("title-bluey", "Bluey", 1);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-failed");
        let snapshot = snapshot_with_job("job-failed", true);

        assert!(!submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
            Some(scryer_domain::TrackedDownloadState::Failed),
            false,
        ));
    }

    #[test]
    fn completed_submission_does_not_block_upgrade_search() {
        let mut item = wanted_episode_item("title-bluey", "Bluey", 1);
        item.grabbed_release = Some("Bluey.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb".to_string());
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-baseline");
        let snapshot = snapshot_with_job("job-baseline", true);

        // A file already occupies the scope, so this download has resolved one
        // way or the other and an upgrade search may proceed.
        assert!(!submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
            None,
            true,
        ));
    }

    /// **D18.** An active download no longer freezes its scope. It becomes a
    /// queued pseudo-incumbent instead, so a genuinely better release can be
    /// grabbed over a slow one and an equal-or-worse one is refused by the
    /// admission ladder with `queued_better_or_equal` — a reason an operator can
    /// read, rather than a silent scope-level skip.
    #[test]
    fn an_active_submission_no_longer_freezes_the_scope() {
        let item = wanted_episode_item("title-bluey", "Bluey", 1);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-upgrade");
        let snapshot = snapshot_with_job("job-upgrade", false);

        assert!(!submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
            None,
            true,
        ));
    }

    /// A held import is a real claim on the scope, so it still counts as
    /// queued — but it no longer *blocks*, which is the behaviour change D18
    /// makes deliberately: a stuck download used to make its scope permanently
    /// unsearchable.
    #[test]
    fn an_import_blocked_submission_no_longer_freezes_the_scope() {
        let item = wanted_episode_item("title-bluey", "Bluey", 1);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-blocked");
        let snapshot = snapshot_with_job("job-blocked", true);

        assert!(!submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
            Some(scryer_domain::TrackedDownloadState::ImportBlocked),
            true,
        ));
    }

    /// The two cases that still hard-skip: a failure the handler has not
    /// processed yet (Sonarr excludes `FailedPending` from its queue spec for
    /// the same reason), and a queue that could not be listed at all — with no
    /// way to build honest pseudo-incumbents, the old whole-scope skip is the
    /// safe answer.
    #[test]
    fn a_failed_pending_submission_and_a_blind_queue_still_hard_skip() {
        let item = wanted_episode_item("title-bluey", "Bluey", 1);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-failed");
        let snapshot = snapshot_with_job("job-failed", true);

        assert!(submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
            Some(scryer_domain::TrackedDownloadState::FailedPending),
            true,
        ));

        let blind = blind_queue_snapshot();
        assert!(submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &blind,
            None,
            true,
        ));
    }

    #[test]
    fn terminal_imported_state_preserves_normal_upgrade_search() {
        let item = wanted_episode_item("title-bluey", "Bluey", 1);
        let episode_id = item.episode_id.as_deref().expect("episode id");
        let submission = episode_submission(&item.title_id, episode_id, "job-imported");
        let snapshot = snapshot_with_job("job-imported", true);

        assert!(!submission_blocks_search_for_wanted_item(
            &submission,
            &item,
            None,
            &snapshot,
            Some(scryer_domain::TrackedDownloadState::Imported),
            true,
        ));
    }

    fn background_acquisition_episode_target(
        title_id: &str,
        season: u32,
        episode: u32,
    ) -> AcquisitionTarget {
        AcquisitionTarget {
            scope_key: format!("{title_id}-s{season}-e{episode}"),
            title_id: title_id.to_string(),
            library_id: "library".to_string(),
            facet: MediaFacet::Series,
            media_type: "episode".to_string(),
            episode_id: Some(format!("{title_id}-s{season}-e{episode}")),
            collection_id: Some(format!("{title_id}-s{season}")),
            series_movie_link_id: None,
            season_number: Some(season.to_string()),
            episode_number: Some(episode.to_string()),
            is_hot: false,
            occupied: false,
        }
    }

    #[test]
    fn background_acquisition_title_queue_enforces_pack_first_order() {
        let targets = vec![
            background_acquisition_episode_target("synthetic-title", 1, 1),
            background_acquisition_episode_target("synthetic-title", 1, 2),
            background_acquisition_episode_target("synthetic-title", 2, 1),
            background_acquisition_episode_target("synthetic-title", 2, 2),
        ];
        let ready_titles = build_background_acquisition_title_work(&targets, &[0, 1, 2, 3]);

        assert_eq!(ready_titles.len(), 1);
        let title_work = ready_titles.front().expect("title work");
        assert_eq!(
            title_work
                .ready
                .iter()
                .map(|work| work.target_index)
                .collect::<Vec<_>>(),
            vec![0, 2, 0, 1, 2, 3]
        );
        assert!(matches!(
            title_work.ready[0].kind,
            BackgroundAcquisitionWorkKind::TitlePack
        ));
        assert!(matches!(
            title_work.ready[1].kind,
            BackgroundAcquisitionWorkKind::SeasonPack { season: 2 }
        ));
        assert!(
            title_work
                .ready
                .iter()
                .skip(2)
                .all(|work| matches!(&work.kind, BackgroundAcquisitionWorkKind::Scope))
        );
    }

    #[test]
    fn background_acquisition_submission_claims_are_atomic_and_route_scoped() {
        let cycle = BackgroundAcquisitionCycleCoordinator::default();
        let route = DownloadRouteKey {
            source_kind: Some(DownloadSourceKind::NzbUrl),
            indexer_id: Some("indexer-a".to_string()),
        };

        assert_eq!(
            cycle.claim_submission(route.clone(), "https://indexer.example/a"),
            SubmissionClaim::Granted
        );
        assert_eq!(
            cycle.claim_submission(route.clone(), "https://indexer.example/a"),
            SubmissionClaim::AlreadyAttempted
        );
        cycle.mark_submitted("https://indexer.example/a");
        assert_eq!(
            cycle.claim_submission(route.clone(), "https://indexer.example/a"),
            SubmissionClaim::AlreadySubmitted
        );
        cycle.mark_failed_route(route.clone());
        assert_eq!(
            cycle.claim_submission(route, "https://indexer.example/b"),
            SubmissionClaim::RouteUnavailable
        );
    }

    #[test]
    fn poisoned_background_acquisition_state_remains_recoverable() {
        let cycle = Arc::new(BackgroundAcquisitionCycleCoordinator::default());
        let poisoned = Arc::clone(&cycle);
        assert!(
            std::thread::spawn(move || {
                let _guard = poisoned.state.lock().expect("test lock");
                panic!("poison the test lock");
            })
            .join()
            .is_err()
        );

        assert_eq!(
            cycle.claim_submission(
                DownloadRouteKey {
                    source_kind: Some(DownloadSourceKind::NzbUrl),
                    indexer_id: Some("indexer-a".to_string()),
                },
                "https://indexer.example/a",
            ),
            SubmissionClaim::Granted
        );
    }

    #[test]
    fn background_acquisition_title_limit_is_four() {
        assert_eq!(BACKGROUND_ACQUISITION_TITLE_LIMIT, 4);
    }
}
