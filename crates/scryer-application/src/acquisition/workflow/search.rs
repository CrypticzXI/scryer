fn parsed_release_season_pack_season(parsed: &crate::ParsedReleaseMetadata) -> Option<u32> {
    parsed.episode.as_ref().and_then(|episode| {
        (episode.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
            && !episode.is_season_extra)
            .then_some(episode.season)
            .flatten()
    })
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
    wanted_item: Option<&AcquisitionScopeState>,
    failed_collection_items: Option<&[AcquisitionScopeState]>,
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
    item: &AcquisitionScopeState,
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
    item: &AcquisitionScopeState,
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
    item: &AcquisitionScopeState,
) -> AppResult<()> {
    app.services
        .workflow
        .acquisition_scope_states
        .update_acquisition_scope_status(
            &item.id,
            AcquisitionScopeStatus::Wanted.as_str(),
            item.last_search_at.as_deref(),
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
        item: &AcquisitionScopeState,
    ) -> AppResult<bool> {
        let decisions = self
            .services
            .workflow
            .acquisition_scope_states
            .list_release_decisions_for_acquisition_scope_state(&item.id, 10)
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
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(wanted_item_id)
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
            .acquisition_scope_states
            .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                statuses: vec!["wanted".into()],
                title_id: Some(title_id.to_string()),
                limit: 500,
                ..AcquisitionScopeStatesQuery::default()
            })
            .await?;

        let mut queued = 0usize;
        for item in &items {
            if !self
                .wanted_item_is_mismatch_recovery_candidate(item)
                .await?
            {
                continue;
            }

            // A rematch changes the scope's fingerprint, so stale coverage is
            // already ignored — the re-open prunes it eagerly and wakes the
            // cursor so recovery starts on the next cycle.
            self.reopen_wanted_scope_for_acquisition(item).await;
            queued += 1;
        }

        Ok(queued)
    }
}
impl AppUseCase {
    async fn queue_monitored_series_items_for_search(
        &self,
        title: &Title,
        _now: &DateTime<Utc>,
    ) -> AppResult<WantedSearchOutcome> {
        self.reopen_series_scopes_for_search(title, None).await
    }
}
impl AppUseCase {
    /// Re-open every fileless monitored episode scope of `title` (optionally
    /// restricted to one season) for acquisition: the derived target set already
    /// contains them; the re-open prunes any coverage so even converged scopes
    /// are searched again on the next cycle (§D5 — a trigger overrides
    /// convergence). Scopes with an in-flight grab are skipped.
    async fn reopen_series_scopes_for_search(
        &self,
        title: &Title,
        season_number: Option<&str>,
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
                if let Some(season) = season_number
                    && episode.season_number.as_deref() != Some(season)
                {
                    continue;
                }

                let item = match self
                    .services
                    .workflow
                    .acquisition_scope_states
                    .get_acquisition_scope_state_for_title(&title.id, Some(&episode.id))
                    .await?
                {
                    Some(item) => {
                        if item.status == AcquisitionScopeStatus::Grabbed {
                            continue;
                        }
                        item
                    }
                    None => self.new_wanted_state_view(
                        title,
                        "episode",
                        Some(episode.id.clone()),
                        None,
                        None,
                        episode.season_number.clone(),
                    ),
                };

                let scheduled = self
                    .reopen_wanted_scope_with_policy(title, &item, SubmissionConflictPolicy::Skip)
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
