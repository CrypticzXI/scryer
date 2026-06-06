const RECENT_QUEUE_PRIORITY_WINDOW_DAYS: i64 = 14;
fn wanted_item_candidates_for_submission_scope(
    title_id: &str,
    scope: &SubmissionScope,
    episodes: &[Episode],
) -> Vec<(WantedItem, Option<String>)> {
    match scope {
        SubmissionScope::Orphan => Vec::new(),
        SubmissionScope::Title => vec![(
            WantedItem {
                id: String::new(),
                title_id: title_id.to_string(),
                title_name: None,
                title_slug: None,
                title_facet: None,
                library_id: None,
                library_name: None,
                library_slug: None,
                episode_id: None,
                collection_id: None,
                season_number: None,
                episode_number: None,
                media_type: "movie".to_string(),
                search_phase: String::new(),
                next_search_at: None,
                last_search_at: None,
                search_count: 0,
                baseline_date: None,
                status: WantedStatus::Wanted,
                grabbed_release: None,
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: String::new(),
                updated_at: String::new(),
            },
            None,
        )],
        SubmissionScope::Episode { episode_id } => {
            let candidate = episodes
                .iter()
                .find(|episode| episode.id == *episode_id)
                .map(|episode| {
                    (
                        wanted_item_candidate_for_episode(title_id, episode),
                        episode.collection_id.clone(),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        wanted_item_candidate_for_episode_id(title_id, episode_id, None, None),
                        None,
                    )
                });
            vec![candidate]
        }
        SubmissionScope::EpisodeSet { episode_ids } => episode_ids
            .iter()
            .map(|episode_id| {
                episodes
                    .iter()
                    .find(|episode| episode.id == *episode_id)
                    .map(|episode| {
                        (
                            wanted_item_candidate_for_episode(title_id, episode),
                            episode.collection_id.clone(),
                        )
                    })
                    .unwrap_or_else(|| {
                        (
                            wanted_item_candidate_for_episode_id(title_id, episode_id, None, None),
                            None,
                        )
                    })
            })
            .collect(),
        SubmissionScope::Collection { collection_id } => {
            let mut candidates = episodes
                .iter()
                .filter(|episode| episode.collection_id.as_deref() == Some(collection_id.as_str()))
                .map(|episode| {
                    (
                        wanted_item_candidate_for_episode(title_id, episode),
                        episode.collection_id.clone(),
                    )
                })
                .collect::<Vec<_>>();
            candidates.push((
                WantedItem {
                    id: String::new(),
                    title_id: title_id.to_string(),
                    title_name: None,
                    title_slug: None,
                    title_facet: None,
                    library_id: None,
                    library_name: None,
                    library_slug: None,
                    episode_id: None,
                    collection_id: Some(collection_id.clone()),
                    season_number: None,
                    episode_number: None,
                    media_type: "interstitial_movie".to_string(),
                    search_phase: String::new(),
                    next_search_at: None,
                    last_search_at: None,
                    search_count: 0,
                    baseline_date: None,
                    status: WantedStatus::Wanted,
                    grabbed_release: None,
                    current_score: None,
                    latest_release_decision: None,
                    mismatch_recovery_eligible: false,
                    created_at: String::new(),
                    updated_at: String::new(),
                },
                Some(collection_id.clone()),
            ));
            candidates
        }
    }
}
fn wanted_item_candidate_for_episode(title_id: &str, episode: &Episode) -> WantedItem {
    wanted_item_candidate_for_episode_id(
        title_id,
        &episode.id,
        episode.collection_id.clone(),
        episode.season_number.clone(),
    )
}
fn wanted_item_candidate_for_episode_id(
    title_id: &str,
    episode_id: &str,
    collection_id: Option<String>,
    season_number: Option<String>,
) -> WantedItem {
    WantedItem {
        id: String::new(),
        title_id: title_id.to_string(),
        title_name: None,
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: Some(episode_id.to_string()),
        collection_id,
        season_number,
        episode_number: None,
        media_type: "episode".to_string(),
        search_phase: String::new(),
        next_search_at: None,
        last_search_at: None,
        search_count: 0,
        baseline_date: None,
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: String::new(),
        updated_at: String::new(),
    }
}
fn submission_for_scope(title_id: &str, scope: &SubmissionScope) -> DownloadSubmission {
    DownloadSubmission {
        title_id: title_id.to_string(),
        facet: String::new(),
        download_client_id: None,
        download_client_type: String::new(),
        download_client_item_id: String::new(),
        source_hint: None,
        source_kind: None,
        source_title: None,
        request_signature: None,
        scope: scope.clone(),
    }
}
fn queue_state_blocks_submission(state: DownloadQueueState) -> bool {
    matches!(
        state,
        DownloadQueueState::Queued
            | DownloadQueueState::Downloading
            | DownloadQueueState::Paused
            | DownloadQueueState::Verifying
            | DownloadQueueState::Repairing
            | DownloadQueueState::Extracting
            | DownloadQueueState::ImportPending
    )
}
fn queue_state_is_replaceable(state: DownloadQueueState) -> bool {
    matches!(
        state,
        DownloadQueueState::Queued | DownloadQueueState::Downloading | DownloadQueueState::Paused
    )
}
fn queue_item_matches_submission(
    item: &DownloadQueueItem,
    submission: &DownloadSubmission,
) -> bool {
    item.download_client_item_id == submission.download_client_item_id
        && submission
            .download_client_id
            .as_deref()
            .map(|client_id| client_id == item.client_id)
            .unwrap_or(true)
}
fn blocking_queue_item_for_submission<'a>(
    queue: &'a [DownloadQueueItem],
    submission: &DownloadSubmission,
) -> Option<&'a DownloadQueueItem> {
    queue.iter().find(|item| {
        queue_item_matches_submission(item, submission) && queue_state_blocks_submission(item.state)
    })
}
impl AppUseCase {
    pub(crate) fn is_recent_for_queue_priority(&self, baseline_date: Option<&str>) -> Option<bool> {
        baseline_date.map(|_| {
            release_is_recent_for_queue_priority(baseline_date, RECENT_QUEUE_PRIORITY_WINDOW_DAYS)
        })
    }
}
impl AppUseCase {
    pub async fn list_title_release_blocklist(
        &self,
        actor: &User,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::View,
        )
        .await?;
        let bounded_limit = limit.clamp(1, 1_000);
        let submissions = self
            .services
            .workflow
            .download_submissions
            .list_for_title(title_id)
            .await
            .unwrap_or_default();
        let episode_ids_by_download_id: HashMap<String, Vec<String>> = submissions
            .into_iter()
            .filter_map(|submission| {
                let episode_ids = submission.scope.episode_ids()?.to_vec();
                if episode_ids.is_empty() {
                    None
                } else {
                    Some((submission.download_client_item_id, episode_ids))
                }
            })
            .collect();
        let entries = self
            .services
            .workflow
            .blocklist_repo
            .list_for_title(title_id, bounded_limit)
            .await?;
        Ok(entries
            .into_iter()
            .map(|entry| {
                let mut episode_ids = blocklist_episode_ids(entry.data_json.as_deref());
                if episode_ids.is_empty()
                    && let Some(download_id) = entry.download_id.as_deref()
                    && let Some(submission_episode_ids) =
                        episode_ids_by_download_id.get(download_id)
                {
                    episode_ids = submission_episode_ids.clone();
                }

                TitleReleaseBlocklistEntry {
                    id: entry.id,
                    source_hint: entry.source_hint,
                    source_title: entry.source_title,
                    error_message: entry.reason,
                    attempted_at: entry.created_at,
                    episode_ids,
                }
            })
            .collect())
    }
}
impl AppUseCase {
    pub async fn clear_title_release_blocklist_entry(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<()> {
        let (entries, _) = self
            .services
            .workflow
            .blocklist_repo
            .list_all(500, 0)
            .await?;
        let entry = entries
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("blocklist entry {id}")))?;
        self.require_title_permission(
            actor,
            &entry.title_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        self.services.workflow.blocklist_repo.remove(id).await
    }
}
impl AppUseCase {
    async fn queue_manual_release_for_title(
        &self,
        actor: &User,
        title: &Title,
        queued_release: QueuedReleaseSelection,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<QueueDownloadOutcome> {
        let QueuedReleaseSelection {
            source_hint,
            source_kind,
            source_title,
        } = queued_release;
        let source_hint_for_attempt = normalize_release_attempt_value(source_hint.as_deref());
        let source_title_for_attempt = normalize_release_attempt_value(source_title.as_deref());
        let request_signature = normalize_release_selection_signature(
            source_hint_for_attempt.as_deref(),
            source_title_for_attempt.as_deref(),
            source_kind,
        );
        let source_password: Option<String> = None;
        let scope_guard = self.lock_download_submission_scope(&title.id, &scope).await;
        let dedupe_guard = self
            .lock_download_submission_signature(&title.id, request_signature.as_deref())
            .await;

        if let Some(signature) = request_signature.as_deref()
            && let Some(existing) = self
                .services
                .workflow
                .download_submissions
                .find_by_title_and_request_signature(&title.id, signature)
                .await?
        {
            let queue = self
                .services
                .integrations
                .download_client
                .list_queue()
                .await?;
            if blocking_queue_item_for_submission(&queue, &existing).is_some() {
                drop(dedupe_guard);
                drop(scope_guard);
                return Ok(QueueDownloadOutcome::Queued(QueuedDownloadResult {
                    job_id: existing.download_client_item_id,
                    queued_release: QueuedReleaseSelection {
                        source_hint,
                        source_kind,
                        source_title,
                    },
                    reused_existing: true,
                }));
            }
        }

        let conflicts = self
            .find_blocking_download_submissions(title, &scope)
            .await?;
        if !conflicts.is_empty() {
            match conflict_policy {
                SubmissionConflictPolicy::Abort | SubmissionConflictPolicy::Skip => {
                    drop(dedupe_guard);
                    drop(scope_guard);
                    return Ok(QueueDownloadOutcome::Conflict(conflicts[0].clone()));
                }
                SubmissionConflictPolicy::ReplaceEarly
                    if conflicts.iter().all(|conflict| conflict.replaceable) =>
                {
                    self.replace_blocking_download_submissions(&conflicts)
                        .await?;
                }
                SubmissionConflictPolicy::ReplaceEarly => {
                    let conflict = conflicts
                        .into_iter()
                        .find(|conflict| !conflict.replaceable)
                        .expect("non-empty conflicts should contain a non-replaceable item");
                    drop(dedupe_guard);
                    drop(scope_guard);
                    return Ok(QueueDownloadOutcome::Conflict(conflict));
                }
            }
        }

        let _ = self
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

        let category = self.derive_download_category(&title.facet).await;
        let is_recent = self.is_recent_for_queue_priority(
            title
                .first_aired
                .as_deref()
                .or(title.digital_release_date.as_deref()),
        );
        let download_id = crate::download_identity::new_download_id();
        let submission_identity = DownloadSubmissionIdentity {
            download_id: Some(download_id.clone()),
        };
        let job_result = self
            .services
            .integrations
            .download_client
            .submit_download(&DownloadClientAddRequest {
                title: title.clone(),
                download_id: Some(download_id),
                source_hint,
                staged_nzb: None,
                source_kind,
                source_title,
                source_password: source_password.clone(),
                category: Some(category),
                queue_priority: None,
                download_directory: None,
                release_title: None,
                indexer_name: None,
                info_hash_hint: None,
                seed_goal_ratio: None,
                seed_goal_seconds: None,
                is_recent,
                season_pack: None,
            })
            .await;

        let grab = match job_result {
            Ok(grab) => {
                {
                    let facet_label = serde_json::to_string(&title.facet)
                        .unwrap_or_else(|_| "\"other\"".to_string())
                        .trim_matches('"')
                        .to_string();
                    metrics::counter!("scryer_grabs_total", "indexer" => "manual", "facet" => facet_label).increment(1);
                }
                let facet_str =
                    serde_json::to_string(&title.facet).unwrap_or_else(|_| "\"other\"".to_string());
                let accepted_identity =
                    crate::download_identity::accepted_download_submission_identity(
                        crate::download_identity::AcceptedDownloadIdentityInput {
                            initial_download_id: submission_identity.download_id.as_deref(),
                            source_kind,
                            source_hint: source_hint_for_attempt.as_deref(),
                            info_hash_hint: None,
                            client_type: Some(grab.client_type.as_str()),
                            client_item_id: Some(grab.job_id.as_str()),
                            accepted_info_hash: grab.info_hash.as_deref(),
                        },
                    );
                let log_download_id = accepted_identity.download_id.clone();
                if let Err(error) = self
                    .services
                    .workflow
                    .download_submissions
                    .record_submission_with_identity(
                        DownloadSubmission {
                            title_id: title.id.clone(),
                            facet: facet_str.trim_matches('"').to_string(),
                            download_client_id: grab.client_id.clone(),
                            download_client_type: grab.client_type.clone(),
                            download_client_item_id: grab.job_id.clone(),
                            source_hint: source_hint_for_attempt.clone(),
                            source_kind,
                            source_title: source_title_for_attempt.clone(),
                            request_signature: request_signature.clone(),
                            scope,
                        },
                        accepted_identity,
                    )
                    .await
                {
                    tracing::warn!(
                        error = %error,
                        client_id = ?grab.client_id,
                        client_type = %grab.client_type,
                        download_client_item_id = %grab.job_id,
                        download_id = ?log_download_id,
                        "download_identity_persistence_failed"
                    );
                    let _ = self
                        .services
                        .workflow
                        .release_attempts
                        .record_release_attempt(
                            Some(title.id.clone()),
                            source_hint_for_attempt.clone(),
                            source_title_for_attempt.clone(),
                            ReleaseDownloadAttemptOutcome::Failed,
                            Some(error.to_string()),
                            source_password.clone(),
                        )
                        .await;
                    return Err(error);
                }
                let _ = self
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
                grab
            }
            Err(error) => {
                let error_message = error.to_string();
                let _ = self
                    .services
                    .workflow
                    .release_attempts
                    .record_release_attempt(
                        Some(title.id.clone()),
                        source_hint_for_attempt.clone(),
                        source_title_for_attempt.clone(),
                        ReleaseDownloadAttemptOutcome::Failed,
                        Some(error_message.clone()),
                        source_password,
                    )
                    .await;
                let blocklist_episode_ids = match &scope {
                    SubmissionScope::Episode { episode_id } => vec![episode_id.clone()],
                    SubmissionScope::EpisodeSet { episode_ids } => episode_ids.clone(),
                    SubmissionScope::Collection { collection_id } => self
                        .services
                        .catalog
                        .shows
                        .list_episodes_for_collection(collection_id)
                        .await
                        .map(|episodes| episodes.into_iter().map(|episode| episode.id).collect())
                        .unwrap_or_default(),
                    SubmissionScope::Title | SubmissionScope::Orphan => Vec::new(),
                };
                let mut blocklist_data = HashMap::new();
                if !blocklist_episode_ids.is_empty() {
                    blocklist_data.insert(
                        "episode_ids".to_string(),
                        serde_json::json!(blocklist_episode_ids),
                    );
                }
                if let SubmissionScope::Collection { collection_id } = &scope {
                    blocklist_data.insert(
                        "collection_id".to_string(),
                        serde_json::json!(collection_id),
                    );
                }
                let _ = self
                    .services
                    .workflow
                    .blocklist_repo
                    .add(&NewBlocklistEntry {
                        title_id: title.id.clone(),
                        source_title: source_title_for_attempt.clone(),
                        source_hint: source_hint_for_attempt.clone(),
                        quality: None,
                        download_id: None,
                        reason: Some(error_message.clone()),
                        data: blocklist_data,
                    })
                    .await;
                drop(dedupe_guard);
                drop(scope_guard);
                return Err(error);
            }
        };

        drop(dedupe_guard);
        drop(scope_guard);

        self.append_domain_event(new_title_domain_event(
            Some(actor.id.clone()),
            title,
            DomainEventPayload::ReleaseGrabbed(ReleaseGrabbedEventData {
                title: title_context_snapshot(title),
                source_title: None,
                source_hint: None,
                download_id: Some(grab.job_id.clone()),
                episode_ids: Vec::new(),
            }),
        ))
        .await?;

        Ok(QueueDownloadOutcome::Queued(QueuedDownloadResult {
            job_id: grab.job_id,
            queued_release: QueuedReleaseSelection {
                source_hint: source_hint_for_attempt,
                source_kind,
                source_title: source_title_for_attempt,
            },
            reused_existing: false,
        }))
    }
}
impl AppUseCase {
    pub async fn add_title_and_queue_download_with_outcome(
        &self,
        actor: &User,
        request: NewTitle,
        queued_release: QueuedReleaseSelection,
    ) -> AppResult<AddTitleAndQueueDownloadOutcome> {
        let library_id = scryer_domain::default_library_id_for_facet(&request.facet);
        self.add_title_and_queue_download_with_outcome_in_library(
            actor,
            request,
            library_id,
            queued_release,
        )
        .await
    }
}
impl AppUseCase {
    pub async fn add_title_and_queue_download_with_outcome_in_library(
        &self,
        actor: &User,
        request: NewTitle,
        library_id: String,
        queued_release: QueuedReleaseSelection,
    ) -> AppResult<AddTitleAndQueueDownloadOutcome> {
        let add_outcome = self
            .add_title_with_outcome_in_library(actor, request, library_id)
            .await?;
        let title = add_outcome.title.clone();
        let queued = self
            .queue_manual_release_for_title(
                actor,
                &title,
                queued_release,
                SubmissionScope::Title,
                SubmissionConflictPolicy::Abort,
            )
            .await?;
        let QueueDownloadOutcome::Queued(queued) = queued else {
            return Err(AppError::Validation(
                "a download is already queued for this title".into(),
            ));
        };

        Ok(AddTitleAndQueueDownloadOutcome {
            title,
            metadata_hydration_state: add_outcome.metadata_hydration_state,
            reused_existing_title: add_outcome.reused_existing_title,
            download_job_id: queued.job_id,
            reused_queued_download: queued.reused_existing,
        })
    }
}
impl AppUseCase {
    pub async fn add_title_and_queue_download(
        &self,
        actor: &User,
        request: NewTitle,
        queued_release: QueuedReleaseSelection,
    ) -> AppResult<(Title, String)> {
        let outcome = self
            .add_title_and_queue_download_with_outcome(actor, request, queued_release)
            .await?;
        Ok((outcome.title, outcome.download_job_id))
    }
}
impl AppUseCase {
    pub async fn queue_existing_title_download(
        &self,
        actor: &User,
        title_id: &str,
        queued_release: QueuedReleaseSelection,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<QueueDownloadOutcome> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        self.queue_manual_release_for_title(actor, &title, queued_release, scope, conflict_policy)
            .await
    }
}
impl AppUseCase {
    pub async fn queue_existing_title_download_from_candidate_token(
        &self,
        actor: &User,
        title_id: &str,
        candidate_token: &str,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<QueueDownloadOutcome> {
        let (queued_release, signed_scope) = self
            .verify_release_candidate_token_for_signed_scope(actor, title_id, candidate_token)
            .await?;
        let outcome = self
            .queue_existing_title_download(
                actor,
                title_id,
                queued_release.clone(),
                signed_scope,
                conflict_policy,
            )
            .await?;
        let _ = scope;
        Ok(match outcome {
            QueueDownloadOutcome::Queued(mut queued) => {
                queued.queued_release = queued_release;
                QueueDownloadOutcome::Queued(queued)
            }
            QueueDownloadOutcome::Conflict(conflict) => QueueDownloadOutcome::Conflict(conflict),
        })
    }
}
impl AppUseCase {
    pub async fn queue_best_release(
        &self,
        actor: &User,
        title_id: &str,
        scope: SubmissionScope,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<QueueDownloadOutcome> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", title_id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let (search_title, subject) = match &scope {
            SubmissionScope::Title | SubmissionScope::Orphan => (
                title.clone(),
                self.resolve_release_search_subject_for_title(&title)
                    .await?,
            ),
            SubmissionScope::Episode { episode_id } => {
                let episode = self
                    .services
                    .catalog
                    .shows
                    .get_episode_by_id(episode_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;
                let season = episode.season_number.clone().ok_or_else(|| {
                    AppError::Validation("episode is missing season number".into())
                })?;
                let episode_number = episode.episode_number.clone().ok_or_else(|| {
                    AppError::Validation("episode is missing episode number".into())
                })?;
                (
                    title.clone(),
                    self.resolve_release_search_subject_for_episode(
                        &title,
                        &season,
                        &episode_number,
                    )
                    .await?,
                )
            }
            SubmissionScope::EpisodeSet { .. } => {
                return Err(AppError::Validation(
                    "best-release search is not supported for multi-episode scopes".into(),
                ));
            }
            SubmissionScope::Collection { collection_id } => {
                let collection = self
                    .services
                    .catalog
                    .shows
                    .get_collection_by_id(collection_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;
                self.resolve_release_search_subject_for_collection(&title, &collection)
                    .await?
            }
        };

        let results = self
            .search_and_evaluate_subject(&search_title, &subject, &actor.id, SearchMode::Auto)
            .await?;
        let best = results
            .into_iter()
            .find(|candidate| candidate.auto_eligible == Some(true))
            .ok_or_else(|| AppError::Validation("no auto-eligible release found".into()))?;
        let queue_scope = if matches!(&scope, SubmissionScope::Collection { .. }) {
            scope
        } else if let Some(parsed) = best.parsed_release_metadata.as_ref() {
            let catalog_episodes = self
                .services
                .catalog
                .shows
                .list_episodes_for_title(&title.id)
                .await
                .unwrap_or_default();
            let catalog_collections = self
                .services
                .catalog
                .shows
                .list_collections_for_title(&title.id)
                .await
                .unwrap_or_default();
            let requested_episode = match &scope {
                SubmissionScope::Episode { episode_id } => catalog_episodes
                    .iter()
                    .find(|episode| episode.id == *episode_id),
                _ => None,
            };
            crate::acquisition_coverage::resolve_release_coverage(
                parsed,
                &catalog_episodes,
                &catalog_collections,
                requested_episode,
            )
            .submission_scope_or(&scope)
        } else {
            scope
        };

        self.queue_existing_title_download(
            actor,
            title_id,
            QueuedReleaseSelection {
                source_hint: best.download_url.clone().or(best.link.clone()),
                source_kind: best.source_kind,
                source_title: Some(best.title.clone()),
            },
            queue_scope,
            conflict_policy,
        )
        .await
    }
}
fn normalize_release_attempt_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
