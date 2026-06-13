fn parsed_release_season_pack_season(parsed: &crate::ParsedReleaseMetadata) -> Option<u32> {
    parsed.episode.as_ref().and_then(|episode| {
        (episode.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
            && !episode.is_season_extra)
            .then_some(episode.season)
            .flatten()
    })
}
fn episode_wanted_schedule_fields(
    baseline_date: Option<&str>,
    now: &DateTime<Utc>,
    immediate: bool,
) -> (Option<String>, String, Option<String>) {
    let normalized_baseline_date = baseline_date
        .filter(|value| parse_schedule_baseline_date(Some(value)).is_some())
        .map(str::to_string);

    let Some(valid_baseline_date) = normalized_baseline_date.clone() else {
        return (None, SearchPhase::PreAir.to_string(), None);
    };

    let schedule = compute_search_schedule(
        "episode",
        Some(valid_baseline_date.as_str()),
        "primary",
        now,
    );
    let next_search_at =
        if immediate && episode_search_window_is_open(Some(valid_baseline_date.as_str()), now) {
            Some(now.to_rfc3339())
        } else {
            Some(schedule.next_search_at)
        };

    (
        Some(valid_baseline_date),
        schedule.search_phase.to_string(),
        next_search_at,
    )
}
fn candidate_is_season_pack_for_season(candidate: &IndexerSearchResult, season_num: u32) -> bool {
    let Some(parsed) = candidate.parsed_release_metadata.as_ref() else {
        return false;
    };

    parsed_release_season_pack_season(parsed) == Some(season_num)
}
#[derive(Clone, Debug, Default)]
struct FailedReleaseAttribution {
    title: Option<Title>,
    episode_ids: Vec<String>,
    collection_id: Option<String>,
}
fn release_quality_hint(source_title: Option<&str>) -> Option<String> {
    source_title.and_then(|title| crate::parse_release_metadata(title).quality)
}
async fn resolve_failed_release_attribution(
    app: &AppUseCase,
    title_id: Option<&str>,
    failed_submission: Option<&DownloadSubmission>,
    wanted_item: Option<&WantedItem>,
    failed_collection_items: Option<&[WantedItem]>,
) -> FailedReleaseAttribution {
    let title = match title_id {
        Some(title_id) => app
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await
            .ok()
            .flatten(),
        None => None,
    };

    let mut attribution = FailedReleaseAttribution {
        title,
        ..Default::default()
    };

    if let Some(submission) = failed_submission {
        if let Some(episode_ids) = submission.scope.episode_ids() {
            for episode_id in episode_ids {
                push_unique_episode_id(&mut attribution.episode_ids, Some(episode_id));
            }
        }
        attribution.collection_id = submission.scope.collection_id().map(str::to_string);
    }

    if let Some(item) = wanted_item {
        push_unique_episode_id(&mut attribution.episode_ids, item.episode_id.as_deref());
        if attribution.collection_id.is_none() {
            attribution.collection_id = item.collection_id.clone();
        }
    }

    if let Some(items) = failed_collection_items {
        for item in items {
            push_unique_episode_id(&mut attribution.episode_ids, item.episode_id.as_deref());
            if attribution.collection_id.is_none() {
                attribution.collection_id = item.collection_id.clone();
            }
        }
    }

    attribution
}
pub(crate) fn download_submission_scope_for_release_title(
    item: &WantedItem,
    episode: Option<&Episode>,
    release_title: &str,
) -> SubmissionScope {
    if item.media_type == "episode" {
        let parsed = crate::parse_release_metadata(release_title);
        if parsed.episode.as_ref().is_some_and(|episode| {
            episode.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
        }) {
            return collection_download_submission_scope_for_wanted_item(item, episode);
        }
    }

    direct_download_submission_scope_for_wanted_item(item, episode)
}
pub(crate) fn submission_blocks_wanted_item(
    submission: &DownloadSubmission,
    item: &WantedItem,
    episode_collection_id: Option<&str>,
) -> bool {
    match &submission.scope {
        SubmissionScope::Orphan => false,
        SubmissionScope::Title => true,
        SubmissionScope::Episode { episode_id } => {
            item.media_type == "episode" && item.episode_id.as_deref() == Some(episode_id.as_str())
        }
        SubmissionScope::EpisodeSet { episode_ids } => {
            item.media_type == "episode"
                && item.episode_id.as_ref().is_some_and(|episode_id| {
                    episode_ids.iter().any(|candidate| candidate == episode_id)
                })
        }
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => {
            item.media_type == "series_movie"
                && item.series_movie_link_id.as_deref() == Some(series_movie_link_id.as_str())
        }
        SubmissionScope::Collection { collection_id } => match item.media_type.as_str() {
            "episode" => episode_collection_id == Some(collection_id.as_str()),
            _ => false,
        },
    }
}
fn resolved_failed_release_hint(failed_submission: Option<&DownloadSubmission>) -> Option<String> {
    failed_submission
        .and_then(|submission| normalize_release_attempt_hint(submission.source_hint.as_deref()))
}
async fn mark_wanted_item_failed_without_reacquire(
    app: &AppUseCase,
    item: &WantedItem,
) -> AppResult<()> {
    app.services
        .workflow
        .wanted_items
        .update_wanted_item_status(
            &item.id,
            WantedStatus::Wanted.as_str(),
            None,
            item.last_search_at.as_deref(),
            item.search_count,
            item.current_score,
            None,
        )
        .await
        .map_err(|err| {
            warn!(
                wanted_item_id = item.id.as_str(),
                title_id = item.title_id.as_str(),
                error = %err,
                "failed to mark wanted item failed without scheduling reacquisition"
            );
            err
        })
}
async fn load_recent_failed_season_pack_seasons_for_title(
    app: &AppUseCase,
    title_id: &str,
    now: &DateTime<Utc>,
) -> HashSet<u32> {
    let cutoff = *now - Duration::minutes(FAILED_GRAB_RESEARCH_COOLDOWN_MINUTES);

    match app
        .services
        .workflow
        .release_attempts
        .list_failed_release_signatures_for_title(title_id, 200)
        .await
    {
        Ok(entries) => entries
            .into_iter()
            .filter_map(|entry| {
                let source_title = entry.source_title?;
                let attempted_at = crate::quality_profile::parse_published_at(&entry.attempted_at)?;
                (attempted_at >= cutoff)
                    .then(|| crate::parse_release_metadata(&source_title))
                    .and_then(|parsed| parsed_release_season_pack_season(&parsed))
            })
            .collect(),
        Err(err) => {
            warn!(
                title_id,
                error = %err,
                "failed to load recent failed season pack attempts"
            );
            HashSet::new()
        }
    }
}
impl AppUseCase {
    async fn wanted_item_is_mismatch_recovery_candidate(
        &self,
        item: &WantedItem,
    ) -> AppResult<bool> {
        let decisions = self
            .services
            .workflow
            .wanted_items
            .list_release_decisions_for_wanted_item(&item.id, 10)
            .await?;
        Ok(!decisions.is_empty()
            && decisions
                .iter()
                .all(|decision| decision.decision_code == "title_mismatch"))
    }
}
impl AppUseCase {
    pub async fn wanted_item_mismatch_recovery_eligible(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<bool> {
        let Some(item) = self
            .services
            .workflow
            .wanted_items
            .get_wanted_item_by_id(wanted_item_id)
            .await?
        else {
            return Ok(false);
        };

        self.wanted_item_is_mismatch_recovery_candidate(&item).await
    }
}
impl AppUseCase {
    pub async fn trigger_title_mismatch_recovery_search(
        &self,
        actor: &User,
        title_id: &str,
    ) -> AppResult<usize> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let items = self
            .services
            .workflow
            .wanted_items
            .list_wanted_items(WantedItemsQuery {
                statuses: vec!["wanted".into()],
                title_id: Some(title_id.to_string()),
                limit: 500,
                ..WantedItemsQuery::default()
            })
            .await?;

        let now = Utc::now();
        let mut queued = 0usize;
        for item in &items {
            if !self
                .wanted_item_is_mismatch_recovery_candidate(item)
                .await?
            {
                continue;
            }

            self.services
                .workflow
                .wanted_items
                .schedule_wanted_item_search(&WantedSearchTransition {
                    id: item.id.clone(),
                    next_search_at: Some(now.to_rfc3339()),
                    last_search_at: item.last_search_at.clone(),
                    search_count: item.search_count,
                    current_score: item.current_score,
                    grabbed_release: item.grabbed_release.clone(),
                })
                .await?;
            queued += 1;
        }

        if queued > 0 {
            self.runtime.acquisition.acquisition_wake.notify_one();
        }

        Ok(queued)
    }
}
impl AppUseCase {
    pub async fn trigger_season_wanted_search(
        &self,
        actor: &User,
        title_id: &str,
        season_number: u32,
    ) -> AppResult<WantedSearchOutcome> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound("title not found".to_string()))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let season_str = season_number.to_string();
        let items = self
            .services
            .workflow
            .wanted_items
            .list_wanted_items(WantedItemsQuery {
                statuses: vec!["wanted".into()],
                media_types: vec!["episode".into()],
                title_id: Some(title_id.to_string()),
                limit: 500,
                ..WantedItemsQuery::default()
            })
            .await?;

        let now = Utc::now();
        let next_search_at = now.to_rfc3339();
        let mut outcome = WantedSearchOutcome::default();
        for item in &items {
            if item.season_number.as_deref() == Some(season_str.as_str()) {
                let scheduled = self
                    .schedule_wanted_item_search_if_unblocked(&title, item, &next_search_at)
                    .await?;
                outcome.queued_count += scheduled.queued_count;
                outcome.skipped_in_progress_count += scheduled.skipped_in_progress_count;
                if outcome.conflict.is_none() {
                    outcome.conflict = scheduled.conflict;
                }
            }
        }

        if outcome.queued_count > 0 {
            self.runtime.acquisition.acquisition_wake.notify_one();
        }

        Ok(outcome)
    }
}
impl AppUseCase {
    async fn schedule_wanted_item_search_if_unblocked(
        &self,
        title: &Title,
        item: &WantedItem,
        next_search_at: &str,
    ) -> AppResult<WantedSearchOutcome> {
        self.schedule_wanted_item_search_with_policy(
            title,
            item,
            next_search_at,
            SubmissionConflictPolicy::Skip,
        )
        .await
    }
}
impl AppUseCase {
    async fn queue_monitored_series_items_for_search(
        &self,
        title: &Title,
        now: &DateTime<Utc>,
    ) -> AppResult<WantedSearchOutcome> {
        let collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await?;

        let existing_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|file| file.role.is_primary())
            .collect::<Vec<_>>();
        let episodes_with_files: std::collections::HashSet<String> = existing_files
            .iter()
            .filter_map(|file| file.episode_id.clone())
            .collect();
        let next_search_at = now.to_rfc3339();
        let mut outcome = WantedSearchOutcome::default();

        for collection in &collections {
            if !collection.monitored {
                continue;
            }

            let episodes = self
                .services
                .catalog
                .shows
                .list_episodes_for_collection(&collection.id)
                .await?;

            for episode in &episodes {
                if !episode.monitored || episodes_with_files.contains(&episode.id) {
                    continue;
                }

                if let Some(item) = self
                    .services
                    .workflow
                    .wanted_items
                    .get_wanted_item_for_title(&title.id, Some(&episode.id))
                    .await?
                {
                    if item.status == WantedStatus::Grabbed {
                        continue;
                    }

                    let scheduled = self
                        .schedule_wanted_item_search_if_unblocked(title, &item, &next_search_at)
                        .await?;
                    outcome.queued_count += scheduled.queued_count;
                    outcome.skipped_in_progress_count += scheduled.skipped_in_progress_count;
                    if outcome.conflict.is_none() {
                        outcome.conflict = scheduled.conflict;
                    }
                    continue;
                }

                let baseline_date = episode.air_date.clone();
                let schedule =
                    compute_search_schedule("episode", baseline_date.as_deref(), "primary", now);
                let item = WantedItem {
                    id: Id::new().0,
                    title_id: title.id.clone(),
                    title_name: None,
                    title_slug: None,
                    title_facet: None,
                    library_id: Some(title.library_id.clone()),
                    library_name: None,
                    library_slug: None,
                    episode_id: Some(episode.id.clone()),
                    collection_id: None,
                    series_movie_link_id: None,
                    season_number: episode.season_number.clone(),
                    episode_number: None,
                    media_type: "episode".to_string(),
                    search_phase: schedule.search_phase.to_string(),
                    next_search_at: Some(next_search_at.clone()),
                    last_search_at: None,
                    search_count: 0,
                    baseline_date,
                    status: WantedStatus::Wanted,
                    grabbed_release: None,
                    current_score: None,
                    latest_release_decision: None,
                    mismatch_recovery_eligible: false,
                    created_at: now.to_rfc3339(),
                    updated_at: now.to_rfc3339(),
                };

                let scheduled = self
                    .ensure_wanted_item_seeded_with_policy(
                        title,
                        &item,
                        SubmissionConflictPolicy::Skip,
                    )
                    .await?;
                outcome.queued_count += scheduled.queued_count;
                outcome.skipped_in_progress_count += scheduled.skipped_in_progress_count;
                if outcome.conflict.is_none() {
                    outcome.conflict = scheduled.conflict;
                }
            }
        }

        Ok(outcome)
    }
}
/// Determine whether a movie has reached its configured availability threshold.
///
/// Returns `true` if the movie should be included in acquisition searches,
/// `false` if it should be skipped because its release dates haven't passed yet.
pub(crate) fn is_movie_available_for_acquisition(
    title: &Title,
    availability: &str,
    now: &DateTime<Utc>,
) -> bool {
    match availability {
        "in_cinemas" => title
            .first_aired
            .as_deref()
            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .map(|date| date <= now.date_naive())
            .unwrap_or(false),
        "released" => {
            if let Some(ref digital) = title.digital_release_date {
                chrono::NaiveDate::parse_from_str(digital, "%Y-%m-%d")
                    .map(|d| d <= now.date_naive())
                    .unwrap_or(false)
            } else if let Some(ref first_aired) = title.first_aired {
                // Fallback: first_aired + 90 days
                chrono::NaiveDate::parse_from_str(first_aired, "%Y-%m-%d")
                    .map(|d| d + chrono::Duration::days(90) <= now.date_naive())
                    .unwrap_or(false)
            } else {
                false
            }
        }
        // "announced" or anything else: always search
        _ => true,
    }
}
