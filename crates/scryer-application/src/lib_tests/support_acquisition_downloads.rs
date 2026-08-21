use super::*;

#[derive(Clone)]
pub(super) struct MockReleaseAttemptRecord {
    pub(super) title_id: Option<String>,
    pub(super) source_hint: Option<String>,
    pub(super) source_title: Option<String>,
    pub(super) outcome: ReleaseDownloadAttemptOutcome,
    pub(super) error_message: Option<String>,
    pub(super) source_password: Option<String>,
    pub(super) attempted_at: String,
}

#[derive(Default)]
pub(super) struct MockReleaseAttemptRepo {
    pub(super) attempts: Arc<Mutex<Vec<MockReleaseAttemptRecord>>>,
}

#[derive(Default)]
pub(super) struct MockBlocklistRepo {
    pub(super) entries: Arc<Mutex<Vec<BlocklistEntry>>>,
}

#[async_trait]
impl ReleaseAttemptRepository for MockReleaseAttemptRepo {
    async fn record_release_attempt(
        &self,
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        outcome: ReleaseDownloadAttemptOutcome,
        error_message: Option<String>,
        source_password: Option<String>,
    ) -> AppResult<()> {
        self.attempts.lock().await.push(MockReleaseAttemptRecord {
            title_id,
            source_hint,
            source_title,
            outcome,
            error_message,
            source_password,
            attempted_at: Utc::now().to_rfc3339(),
        });
        Ok(())
    }

    async fn list_failed_release_signatures(
        &self,
        limit: usize,
    ) -> AppResult<Vec<ReleaseDownloadFailureSignature>> {
        let mut attempts: Vec<_> = self
            .attempts
            .lock()
            .await
            .iter()
            .filter(|attempt| attempt.outcome == ReleaseDownloadAttemptOutcome::Failed)
            .cloned()
            .collect();
        attempts.sort_by(|left, right| right.attempted_at.cmp(&left.attempted_at));
        let mut seen = HashSet::new();
        let mut deduped = Vec::new();
        for attempt in attempts {
            let Some(normalized_title) =
                crate::normalize_release_attempt_title(attempt.source_title.as_deref())
            else {
                continue;
            };
            if seen.insert(normalized_title) {
                deduped.push(ReleaseDownloadFailureSignature {
                    source_hint: attempt.source_hint,
                    source_title: attempt.source_title,
                });
            }
            if deduped.len() >= limit {
                break;
            }
        }

        Ok(deduped)
    }

    async fn list_failed_release_signatures_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
        let mut attempts: Vec<_> = self
            .attempts
            .lock()
            .await
            .iter()
            .filter(|attempt| {
                attempt.outcome == ReleaseDownloadAttemptOutcome::Failed
                    && attempt.title_id.as_deref() == Some(title_id)
            })
            .cloned()
            .collect();
        attempts.sort_by(|left, right| right.attempted_at.cmp(&left.attempted_at));
        let mut seen = HashSet::new();
        let mut deduped = Vec::new();
        for attempt in attempts {
            let Some(normalized_title) =
                crate::normalize_release_attempt_title(attempt.source_title.as_deref())
            else {
                continue;
            };
            if seen.insert(normalized_title) {
                deduped.push(TitleReleaseBlocklistEntry {
                    id: format!(
                        "failed-attempt:{}:{}:{}",
                        attempt.attempted_at,
                        attempt.source_title.as_deref().unwrap_or_default(),
                        attempt.source_hint.as_deref().unwrap_or_default(),
                    ),
                    source_hint: attempt.source_hint,
                    source_title: attempt.source_title,
                    error_message: attempt.error_message,
                    attempted_at: attempt.attempted_at,
                    episode_ids: Vec::new(),
                });
            }
            if deduped.len() >= limit {
                break;
            }
        }

        Ok(deduped)
    }

    async fn get_latest_source_password(
        &self,
        title_id: Option<&str>,
        source_hint: Option<&str>,
        source_title: Option<&str>,
    ) -> AppResult<Option<String>> {
        Ok(self
            .attempts
            .lock()
            .await
            .iter()
            .rev()
            .find(|attempt| {
                title_id.is_none_or(|title_id| attempt.title_id.as_deref() == Some(title_id))
                    && source_hint.is_none_or(|source_hint| {
                        attempt.source_hint.as_deref() == Some(source_hint)
                    })
                    && source_title.is_none_or(|source_title| {
                        attempt.source_title.as_deref() == Some(source_title)
                    })
            })
            .and_then(|attempt| attempt.source_password.clone()))
    }
}

#[async_trait]
impl BlocklistRepository for MockBlocklistRepo {
    async fn add(&self, entry: &NewBlocklistEntry) -> AppResult<String> {
        let id = Id::new().0;
        let data_json = if entry.data.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&entry.data)
                    .map_err(|err| AppError::Repository(err.to_string()))?,
            )
        };
        self.entries.lock().await.push(BlocklistEntry {
            id: id.clone(),
            title_id: entry.title_id.clone(),
            source_title: entry.source_title.clone(),
            source_hint: entry.source_hint.clone(),
            quality: entry.quality.clone(),
            download_id: entry.download_id.clone(),
            reason: entry.reason.clone(),
            data_json,
            created_at: Utc::now().to_rfc3339(),
        });
        Ok(id)
    }

    async fn list_for_title(&self, title_id: &str, limit: usize) -> AppResult<Vec<BlocklistEntry>> {
        let mut entries: Vec<_> = self
            .entries
            .lock()
            .await
            .iter()
            .filter(|entry| entry.title_id == title_id)
            .cloned()
            .collect();
        entries.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        entries.truncate(limit);
        Ok(entries)
    }

    async fn list_all(&self, limit: usize, offset: usize) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        let mut entries = self.entries.lock().await.clone();
        entries.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        let total = entries.len() as i64;
        let page = entries.into_iter().skip(offset).take(limit).collect();
        Ok((page, total))
    }

    async fn has_recorded_download_failure(
        &self,
        title_id: &str,
        source_title: Option<&str>,
    ) -> AppResult<bool> {
        let entries = self.entries.lock().await;
        let normalized_source_title = source_title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase);
        if normalized_source_title.is_none() {
            return Ok(false);
        }

        Ok(entries.iter().any(|entry| {
            entry.title_id == title_id && entry.source_title == normalized_source_title
        }))
    }

    async fn remove(&self, id: &str) -> AppResult<()> {
        self.entries.lock().await.retain(|entry| entry.id != id);
        Ok(())
    }

    async fn is_blocklisted(&self, title_id: &str, source_title: &str) -> AppResult<bool> {
        Ok(self.entries.lock().await.iter().any(|entry| {
            entry.title_id == title_id
                && entry
                    .source_title
                    .as_deref()
                    .is_some_and(|value| value == source_title)
        }))
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.entries
            .lock()
            .await
            .retain(|entry| entry.title_id != title_id);
        Ok(())
    }
}

/// The title's per-title release blocklist entries as the app sees them.
pub(super) async fn title_blocklist_entries(
    app: &AppUseCase,
    title_id: &str,
) -> Vec<BlocklistEntry> {
    app.services
        .workflow
        .blocklist_repo
        .list_for_title(title_id, 50)
        .await
        .expect("list title blocklist entries")
}

#[derive(Default, Clone)]
pub(super) struct TrackingDownloadSubmissionRepo {
    pub(super) store: Arc<Mutex<Vec<DownloadSubmission>>>,
    pub(super) record_submission_error: Arc<Mutex<Option<String>>>,
    pub(super) identities: DownloadSubmissionIdentities,
    pub(super) identity_states: DownloadIdentityStates,
    pub(super) tracked_states: TrackedDownloadStates,
    pub(super) deleted_title_ids: Arc<Mutex<Vec<String>>>,
    pub(super) list_for_title_calls: Arc<Mutex<Vec<String>>>,
}

#[derive(Default, Clone)]
pub(super) struct TrackingAcquisitionScopeStateRepo {
    pub(super) store: Arc<Mutex<Vec<AcquisitionScopeState>>>,
    pub(super) release_decisions: Arc<Mutex<Vec<ReleaseDecision>>>,
    pub(super) title_facets: Arc<Mutex<HashMap<String, MediaFacet>>>,
    pub(super) status_update_calls: Arc<Mutex<Vec<String>>>,
}

impl TrackingAcquisitionScopeStateRepo {
    pub(super) async fn remember_title_facet(&self, title_id: &str, facet: MediaFacet) {
        self.title_facets
            .lock()
            .await
            .insert(title_id.to_string(), facet);
    }

    pub(super) async fn status_update_call_count_for(&self, id: &str) -> usize {
        self.status_update_calls
            .lock()
            .await
            .iter()
            .filter(|existing| existing.as_str() == id)
            .count()
    }
}

#[derive(Clone)]
pub(super) struct TrackingAcquisitionStateRepo {
    pub(super) download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pub(super) pending_releases: Arc<TrackingPendingReleaseRepo>,
    pub(super) acquisition_scope_states: Arc<TrackingAcquisitionScopeStateRepo>,
}

#[async_trait]
impl AcquisitionScopeStateRepository for TrackingAcquisitionScopeStateRepo {
    async fn upsert_acquisition_scope_state(
        &self,
        item: &AcquisitionScopeState,
    ) -> AppResult<String> {
        let mut store = self.store.lock().await;
        if let Some(existing) = store.iter_mut().find(|existing| existing.id == item.id) {
            *existing = item.clone();
        } else {
            store.push(item.clone());
        }
        Ok(item.id.clone())
    }

    async fn update_acquisition_scope_status(
        &self,
        id: &str,
        status: &str,
        last_search_at: Option<&str>,
        grabbed_release: Option<&str>,
    ) -> AppResult<()> {
        let mut store = self.store.lock().await;
        let item = store
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| AppError::NotFound(format!("wanted item {id}")))?;
        item.status = AcquisitionScopeStatus::parse(status)
            .ok_or_else(|| AppError::Repository(format!("invalid wanted status {status}")))?;
        item.last_search_at = last_search_at.map(str::to_string);
        item.grabbed_release = grabbed_release.map(str::to_string);
        item.updated_at = Utc::now().to_rfc3339();
        drop(store);
        self.status_update_calls.lock().await.push(id.to_string());
        Ok(())
    }

    async fn record_acquisition_scope_search_attempt(
        &self,
        id: &str,
        last_search_at: &str,
    ) -> AppResult<()> {
        let mut store = self.store.lock().await;
        let item = store
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| AppError::NotFound(format!("wanted item {id}")))?;
        item.last_search_at = Some(last_search_at.to_string());
        item.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    async fn get_acquisition_scope_state_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<AcquisitionScopeState>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .find(|item| item.title_id == title_id && item.episode_id.as_deref() == episode_id)
            .cloned())
    }

    async fn delete_acquisition_scope_states_for_title(&self, title_id: &str) -> AppResult<()> {
        self.store
            .lock()
            .await
            .retain(|item| item.title_id != title_id);
        Ok(())
    }

    async fn delete_acquisition_scope_states_for_collection(
        &self,
        collection_id: &str,
    ) -> AppResult<()> {
        self.store
            .lock()
            .await
            .retain(|item| item.collection_id.as_deref() != Some(collection_id));
        Ok(())
    }

    async fn delete_acquisition_scope_states_for_series_movie_link(
        &self,
        series_movie_link_id: &str,
    ) -> AppResult<()> {
        self.store
            .lock()
            .await
            .retain(|item| item.series_movie_link_id.as_deref() != Some(series_movie_link_id));
        Ok(())
    }

    async fn delete_acquisition_scope_states_for_episode(&self, episode_id: &str) -> AppResult<()> {
        self.store
            .lock()
            .await
            .retain(|item| item.episode_id.as_deref() != Some(episode_id));
        Ok(())
    }

    async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String> {
        self.release_decisions.lock().await.push(decision.clone());
        Ok(decision.id.clone())
    }

    async fn get_acquisition_scope_state_by_id(
        &self,
        id: &str,
    ) -> AppResult<Option<AcquisitionScopeState>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .find(|item| item.id == id)
            .cloned())
    }

    async fn list_acquisition_scope_states(
        &self,
        query: AcquisitionScopeStatesQuery,
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        let AcquisitionScopeStatesQuery {
            statuses,
            media_types,
            title_id,
            title_search,
            latest_decision_codes,
            limit,
            offset,
            library_ids: _,
        } = query;
        let latest_decisions = self.release_decisions.lock().await.clone();
        let normalized_title_search = title_search.as_deref().map(str::to_lowercase);
        let items: Vec<AcquisitionScopeState> = self
            .store
            .lock()
            .await
            .iter()
            .filter(|item| {
                let latest_decision = latest_decisions
                    .iter()
                    .filter(|decision| decision.wanted_item_id == item.id)
                    .max_by(|left, right| left.created_at.cmp(&right.created_at));
                (statuses.is_empty()
                    || statuses.iter().any(|status| item.status.as_str() == status))
                    && (media_types.is_empty() || media_types.contains(&item.media_type))
                    && title_id
                        .as_deref()
                        .is_none_or(|title_id| item.title_id == title_id)
                    && normalized_title_search.as_ref().is_none_or(|title_search| {
                        item.title_name.as_deref().is_some_and(|title_name| {
                            title_name.to_lowercase().contains(title_search)
                        })
                    })
                    && (latest_decision_codes.is_empty()
                        || latest_decision_codes.iter().any(|code| {
                            latest_decision
                                .as_ref()
                                .is_some_and(|decision| decision.decision_code == *code)
                        }))
            })
            .skip(offset.max(0) as usize)
            .take(limit.max(0) as usize)
            .cloned()
            .collect();
        Ok(items)
    }

    async fn count_acquisition_scope_states(
        &self,
        query: AcquisitionScopeStatesQuery,
    ) -> AppResult<i64> {
        let AcquisitionScopeStatesQuery {
            statuses,
            media_types,
            title_id,
            title_search,
            latest_decision_codes,
            ..
        } = query;
        let latest_decisions = self.release_decisions.lock().await.clone();
        let normalized_title_search = title_search.as_deref().map(str::to_lowercase);
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|item| {
                let latest_decision = latest_decisions
                    .iter()
                    .filter(|decision| decision.wanted_item_id == item.id)
                    .max_by(|left, right| left.created_at.cmp(&right.created_at));
                (statuses.is_empty()
                    || statuses.iter().any(|status| item.status.as_str() == status))
                    && (media_types.is_empty() || media_types.contains(&item.media_type))
                    && title_id
                        .as_deref()
                        .is_none_or(|title_id| item.title_id == title_id)
                    && normalized_title_search.as_ref().is_none_or(|title_search| {
                        item.title_name.as_deref().is_some_and(|title_name| {
                            title_name.to_lowercase().contains(title_search)
                        })
                    })
                    && (latest_decision_codes.is_empty()
                        || latest_decision_codes.iter().any(|code| {
                            latest_decision
                                .as_ref()
                                .is_some_and(|decision| decision.decision_code == *code)
                        }))
            })
            .count() as i64)
    }

    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        Ok(self
            .release_decisions
            .lock()
            .await
            .iter()
            .filter(|decision| decision.title_id == title_id)
            .skip(offset.max(0) as usize)
            .take(limit.max(0) as usize)
            .cloned()
            .collect())
    }

    async fn list_release_decisions_for_acquisition_scope_state(
        &self,
        wanted_item_id: &str,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        Ok(self
            .release_decisions
            .lock()
            .await
            .iter()
            .filter(|decision| decision.wanted_item_id == wanted_item_id)
            .skip(offset.max(0) as usize)
            .take(limit.max(0) as usize)
            .cloned()
            .collect())
    }
    async fn count_release_decisions_for_title(&self, title_id: &str) -> AppResult<i64> {
        Ok(self
            .release_decisions
            .lock()
            .await
            .iter()
            .filter(|decision| decision.title_id == title_id)
            .count() as i64)
    }

    async fn count_release_decisions_for_acquisition_scope_state(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<i64> {
        Ok(self
            .release_decisions
            .lock()
            .await
            .iter()
            .filter(|decision| decision.wanted_item_id == wanted_item_id)
            .count() as i64)
    }
}

#[async_trait]
impl AcquisitionStateRepository for TrackingAcquisitionStateRepo {
    async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()> {
        if let Some(identity) = commit.download_submission_identity.clone() {
            self.download_submissions
                .record_submission_with_identity(commit.download_submission.clone(), identity)
                .await?;
        } else {
            self.download_submissions
                .record_submission(commit.download_submission.clone())
                .await?;
        }

        let mut covered_wanted_item_ids = commit.covered_wanted_item_ids.clone();
        if !covered_wanted_item_ids
            .iter()
            .any(|id| id == &commit.wanted_item_id)
        {
            covered_wanted_item_ids.push(commit.wanted_item_id.clone());
        }
        covered_wanted_item_ids.sort();
        covered_wanted_item_ids.dedup();
        for wanted_item_id in &covered_wanted_item_ids {
            self.acquisition_scope_states
                .update_acquisition_scope_status(
                    wanted_item_id,
                    AcquisitionScopeStatus::Grabbed.as_str(),
                    commit.last_search_at.as_deref(),
                    Some(&commit.grabbed_release),
                )
                .await?;
        }

        if let Some(pending_release_id) = commit.grabbed_pending_release_id.as_deref() {
            self.pending_releases
                .update_pending_release_status(
                    pending_release_id,
                    PendingReleaseStatus::Grabbed,
                    commit.grabbed_at.as_deref(),
                )
                .await?;
        }

        let mut store = self.pending_releases.store.lock().await;
        for release in store.iter_mut() {
            let is_sibling = covered_wanted_item_ids
                .iter()
                .any(|wanted_item_id| wanted_item_id == &release.wanted_item_id)
                && commit
                    .grabbed_pending_release_id
                    .as_deref()
                    .is_none_or(|pending_release_id| release.id != pending_release_id);
            let should_supersede = matches!(
                release.status,
                PendingReleaseStatus::Waiting | PendingReleaseStatus::Standby
            );
            if is_sibling && should_supersede {
                release.status = PendingReleaseStatus::Superseded;
            }
        }

        Ok(())
    }
}

pub(super) fn download_submission_key(submission: &DownloadSubmission) -> TrackedDownloadStateKey {
    (
        submission
            .download_client_id
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_string(),
        submission.download_client_type.clone(),
        submission.download_client_item_id.clone(),
    )
}

pub(super) fn download_source_identity_key(
    identity: &DownloadSourceIdentity,
) -> TrackedDownloadStateKey {
    (
        identity
            .client_id
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_string(),
        identity.client_type.clone(),
        identity.item_id.clone(),
    )
}

pub(super) fn download_identity_state_key(
    identity: &DownloadSubmissionIdentity,
    source_identity: Option<&DownloadSourceIdentity>,
) -> Option<String> {
    let download_id = identity
        .download_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)?;

    if download_id.starts_with("scryer-download:")
        || (matches!(download_id.len(), 40 | 64)
            && download_id.chars().all(|ch| ch.is_ascii_hexdigit()))
    {
        return Some(format!("download:{download_id}"));
    }

    let source_identity = source_identity?;
    let client_type = source_identity.client_type.trim();
    if client_type.is_empty() {
        return None;
    }

    Some(format!(
        "client:{}:{}:download:{}",
        source_identity
            .client_id
            .as_deref()
            .map(str::trim)
            .unwrap_or_default(),
        client_type.to_ascii_lowercase(),
        download_id
    ))
}

#[async_trait]
impl DownloadSubmissionRepository for TrackingDownloadSubmissionRepo {
    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        if let Some(message) = self.record_submission_error.lock().await.clone() {
            return Err(AppError::Repository(message));
        }
        let mut entries = self.store.lock().await;
        if let Some(existing) = entries.iter_mut().find(|entry| {
            entry.download_client_id == submission.download_client_id
                && entry.download_client_type == submission.download_client_type
                && entry.download_client_item_id == submission.download_client_item_id
        }) {
            *existing = submission;
        } else {
            entries.push(submission);
        }
        Ok(())
    }

    async fn record_submission_with_identity(
        &self,
        submission: DownloadSubmission,
        submission_identity: DownloadSubmissionIdentity,
    ) -> AppResult<()> {
        let identity = DownloadSourceIdentity::from_submission(&submission);
        self.record_submission(submission).await?;
        self.record_submission_identity(&identity, &submission_identity)
            .await
    }

    async fn record_submission_identity(
        &self,
        identity: &DownloadSourceIdentity,
        submission_identity: &DownloadSubmissionIdentity,
    ) -> AppResult<()> {
        let key = download_source_identity_key(identity);
        let mut identities = self.identities.lock().await;
        let previous = identities.insert(key.clone(), submission_identity.clone());
        if previous.as_ref() != Some(submission_identity) {
            self.tracked_states.lock().await.remove(&key);
        }
        Ok(())
    }

    async fn find_by_client_item_id(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<DownloadSubmission>> {
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .find(|entry| {
                entry.download_client_id.as_deref().unwrap_or("").trim()
                    == identity.client_id.as_deref().unwrap_or("")
                    && entry.download_client_type == identity.client_type.as_str()
                    && entry.download_client_item_id == identity.item_id.as_str()
            })
            .cloned())
    }

    async fn list_by_download_id(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_id: &str,
    ) -> AppResult<Vec<DownloadSubmission>> {
        let keys = self
            .identities
            .lock()
            .await
            .iter()
            .filter(|(key, identity)| {
                key.0.as_str() == client_id.unwrap_or("")
                    && key.1.eq_ignore_ascii_case(client_type)
                    && identity.download_id.as_deref() == Some(download_id)
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .filter(|entry| {
                keys.iter()
                    .any(|key| *key == download_submission_key(entry))
            })
            .cloned()
            .collect())
    }

    async fn get_submission_identity(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<DownloadSubmissionIdentity>> {
        Ok(self
            .identities
            .lock()
            .await
            .get(&download_source_identity_key(identity))
            .cloned())
    }

    async fn record_identity_tracked_state(
        &self,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
        tracked_state: &str,
        _reason: Option<&str>,
        _detail: Option<&str>,
    ) -> AppResult<()> {
        if let Some(key) = download_identity_state_key(identity, source_identity) {
            self.identity_states
                .lock()
                .await
                .insert(key, tracked_state.to_string());
        }
        Ok(())
    }

    async fn get_identity_tracked_state(
        &self,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
    ) -> AppResult<Option<String>> {
        let Some(key) = download_identity_state_key(identity, source_identity) else {
            return Ok(None);
        };
        Ok(self.identity_states.lock().await.get(&key).cloned())
    }

    async fn list_for_client_items(
        &self,
        client_items: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<DownloadSubmission>> {
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .filter(|entry| {
                client_items.iter().any(|identity| {
                    entry.download_client_id.as_deref().unwrap_or("").trim()
                        == identity.client_id.as_deref().unwrap_or("")
                        && entry.download_client_type == identity.client_type.as_str()
                        && entry.download_client_item_id == identity.item_id.as_str()
                })
            })
            .cloned()
            .collect())
    }

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
        self.list_for_title_calls
            .lock()
            .await
            .push(title_id.to_string());
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .filter(|entry| entry.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn find_by_title_and_request_signature(
        &self,
        title_id: &str,
        request_signature: &str,
        purpose: DownloadSubmissionPurpose,
        scope: &SubmissionScope,
    ) -> AppResult<Option<DownloadSubmission>> {
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .find(|entry| {
                entry.title_id == title_id
                    && entry.request_signature.as_deref() == Some(request_signature)
                    && entry.purpose == purpose
                    && &entry.scope == scope
            })
            .cloned())
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.deleted_title_ids
            .lock()
            .await
            .push(title_id.to_string());
        let removed_keys: Vec<_> = self
            .store
            .lock()
            .await
            .iter()
            .filter(|entry| entry.title_id == title_id)
            .map(download_submission_key)
            .collect();
        self.store
            .lock()
            .await
            .retain(|entry| entry.title_id != title_id);
        self.tracked_states
            .lock()
            .await
            .retain(|key, _| !removed_keys.iter().any(|removed| removed == key));
        self.identities
            .lock()
            .await
            .retain(|key, _| !removed_keys.iter().any(|removed| removed == key));
        Ok(())
    }

    async fn delete_by_client_item_id(&self, identity: &DownloadSourceIdentity) -> AppResult<()> {
        let key = download_source_identity_key(identity);
        self.store.lock().await.retain(|entry| {
            entry.download_client_id.as_deref().unwrap_or("").trim()
                != identity.client_id.as_deref().unwrap_or("")
                || entry.download_client_type != identity.client_type.as_str()
                || entry.download_client_item_id != identity.item_id.as_str()
        });
        self.tracked_states.lock().await.remove(&key);
        self.identities.lock().await.remove(&key);
        Ok(())
    }

    async fn update_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
        tracked_state: &str,
    ) -> AppResult<()> {
        let key = download_source_identity_key(identity);
        self.tracked_states
            .lock()
            .await
            .insert(key, tracked_state.to_string());

        let mut entries = self.store.lock().await;
        if !entries.iter().any(|entry| {
            entry.download_client_id.as_deref().unwrap_or("").trim()
                == identity.client_id.as_deref().unwrap_or("")
                && entry.download_client_type == identity.client_type.as_str()
                && entry.download_client_item_id == identity.item_id.as_str()
        }) {
            entries.push(DownloadSubmission {
                title_id: String::new(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: String::new(),
                download_client_id: identity.client_id.clone(),
                download_client_type: identity.client_type.clone(),
                download_client_item_id: identity.item_id.clone(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: None,
                release_size_bytes: None,
                request_signature: None,
                scope: SubmissionScope::Orphan,
            });
        }
        Ok(())
    }

    async fn get_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<String>> {
        Ok(self
            .tracked_states
            .lock()
            .await
            .get(&download_source_identity_key(identity))
            .cloned())
    }
}

#[derive(Default, Clone)]
pub(super) struct TrackingPendingReleaseRepo {
    pub(super) store: Arc<Mutex<Vec<PendingRelease>>>,
    pub(super) deleted_title_ids: Arc<Mutex<Vec<String>>>,
    pub(super) delete_error: Arc<Mutex<Option<String>>>,
}

impl TrackingPendingReleaseRepo {
    pub(super) async fn fail_delete_for_title(&self, message: &str) {
        *self.delete_error.lock().await = Some(message.to_string());
    }
}

#[async_trait]
impl PendingReleaseRepository for TrackingPendingReleaseRepo {
    async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String> {
        self.store.lock().await.push(release.clone());
        Ok(release.id.clone())
    }

    async fn list_expired_pending_releases(&self, now: &str) -> AppResult<Vec<PendingRelease>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|release| {
                release.status == PendingReleaseStatus::Waiting
                    && release.delay_until.as_str() <= now
            })
            .cloned()
            .collect())
    }

    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|release| release.status.is_open_for_review())
            .cloned()
            .collect())
    }

    async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .find(|release| release.id == id)
            .cloned())
    }

    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|release| {
                release.wanted_item_id == wanted_item_id
                    && release.status == PendingReleaseStatus::Waiting
            })
            .cloned()
            .collect())
    }

    async fn list_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|release| release.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn list_pending_releases_page(
        &self,
        query: PendingReleasesPageQuery,
    ) -> AppResult<(Vec<PendingRelease>, i64)> {
        let mut matched = self
            .store
            .lock()
            .await
            .iter()
            .filter(|release| release.status.is_open_for_review())
            .filter(|release| {
                query
                    .title_id
                    .as_deref()
                    .is_none_or(|title_id| release.title_id == title_id)
            })
            .filter(|release| {
                query
                    .wanted_item_id
                    .as_deref()
                    .is_none_or(|wanted_item_id| release.wanted_item_id == wanted_item_id)
            })
            .filter(|release| {
                query.statuses.is_empty()
                    || query
                        .statuses
                        .iter()
                        .any(|status| status.as_str() == release.status.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        match query.sort {
            PendingReleasePageSort::DelayUntilAsc => matched.sort_by(|a, b| {
                a.delay_until
                    .cmp(&b.delay_until)
                    .then_with(|| a.id.cmp(&b.id))
            }),
            PendingReleasePageSort::ReleaseScoreDesc => matched.sort_by(|a, b| {
                b.release_score
                    .cmp(&a.release_score)
                    .then_with(|| a.delay_until.cmp(&b.delay_until))
                    .then_with(|| a.id.cmp(&b.id))
            }),
        }
        let total = matched.len() as i64;
        let offset = query.offset.max(0) as usize;
        let limit = query.limit.max(0) as usize;
        let page = matched.into_iter().skip(offset).take(limit).collect();
        Ok((page, total))
    }

    async fn update_pending_release_status(
        &self,
        id: &str,
        status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        if let Some(release) = self
            .store
            .lock()
            .await
            .iter_mut()
            .find(|release| release.id == id)
        {
            release.status = status;
            release.grabbed_at = grabbed_at.map(str::to_string);
        }
        Ok(())
    }

    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|release| {
                release.wanted_item_id == wanted_item_id
                    && release.status == PendingReleaseStatus::Standby
            })
            .cloned()
            .collect())
    }

    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        self.store.lock().await.retain(|release| {
            !(release.wanted_item_id == wanted_item_id
                && release.status == PendingReleaseStatus::Standby)
        });
        Ok(())
    }

    async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|release| release.status == PendingReleaseStatus::Standby)
            .cloned()
            .collect())
    }

    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: PendingReleaseStatus,
        next_status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        let mut store = self.store.lock().await;
        let Some(release) = store.iter_mut().find(|release| release.id == id) else {
            return Ok(false);
        };
        if release.status != current_status {
            return Ok(false);
        }
        release.status = next_status;
        release.grabbed_at = grabbed_at.map(str::to_string);
        Ok(true)
    }

    async fn supersede_pending_releases_for_acquisition_scope_state(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()> {
        for release in self.store.lock().await.iter_mut() {
            if release.wanted_item_id == wanted_item_id
                && release.id != except_id
                && release.status == PendingReleaseStatus::Waiting
            {
                release.status = PendingReleaseStatus::Superseded;
            }
        }
        Ok(())
    }

    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        if let Some(message) = self.delete_error.lock().await.clone() {
            return Err(AppError::Repository(message));
        }
        self.deleted_title_ids
            .lock()
            .await
            .push(title_id.to_string());
        self.store
            .lock()
            .await
            .retain(|release| release.title_id != title_id);
        Ok(())
    }
}

#[derive(Default, Clone)]
pub(super) struct TrackingHousekeepingRepo {
    pub(super) operation_log: Arc<Mutex<Vec<String>>>,
}

impl TrackingHousekeepingRepo {
    pub(super) fn with_operation_log(operation_log: Arc<Mutex<Vec<String>>>) -> Self {
        Self { operation_log }
    }
}

#[async_trait]
impl HousekeepingRepository for TrackingHousekeepingRepo {
    async fn delete_release_decisions_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_release_attempts_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_history_events_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_domain_events_older_than_for_types(
        &self,
        _days: i64,
        _event_types: &[DomainEventType],
    ) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_title_history_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_download_import_artifacts_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_terminal_imports_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_terminal_download_queue_commands_older_than(
        &self,
        _days: i64,
    ) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_rule_set_history_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_history_events_for_title_ids(&self, _title_ids: &[String]) -> AppResult<u32> {
        self.operation_log
            .lock()
            .await
            .push("delete_history_events".to_string());
        Ok(0)
    }

    async fn delete_download_import_artifacts_for_title_ids(
        &self,
        _title_ids: &[String],
    ) -> AppResult<u32> {
        self.operation_log
            .lock()
            .await
            .push("delete_download_import_artifacts".to_string());
        Ok(0)
    }

    async fn delete_release_attempts_for_title_ids(&self, _title_ids: &[String]) -> AppResult<u32> {
        self.operation_log
            .lock()
            .await
            .push("delete_release_attempts".to_string());
        Ok(0)
    }

    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>> {
        Ok(Vec::new())
    }

    async fn list_media_files_with_roots(
        &self,
    ) -> AppResult<Vec<crate::HousekeepingMediaFileRootRow>> {
        Ok(Vec::new())
    }

    async fn delete_media_files_by_ids(&self, _ids: &[String]) -> AppResult<u32> {
        Ok(0)
    }

    async fn prune_unreferenced_title_image_blobs(&self, _limit: u32) -> AppResult<u32> {
        Ok(0)
    }
}

#[derive(Clone)]
pub(super) enum StubSubmitError {
    SubmitUnavailable(String),
    /// The router exhausted every prioritized client (typed).
    FailoverExhausted(String),
    /// A plain repository error — including the exact text the router used to
    /// emit before failover exhaustion was typed. Never failover evidence.
    Repository(String),
    Validation(String),
    Rejected(String),
    Ambiguous(String),
}

/// The exact message the router produced before `DownloadSubmitFailoverExhausted`
/// existed; paths must treat it as a definitive failure now.
pub(super) const LEGACY_FAILOVER_REPOSITORY_MESSAGE: &str =
    "all prioritized download clients failed to enqueue this release";

impl StubSubmitError {
    pub(super) fn into_app_error(self) -> AppError {
        match self {
            Self::SubmitUnavailable(message) => AppError::download_submit_unavailable(message),
            Self::FailoverExhausted(message) => {
                AppError::download_submit_failover_exhausted(message)
            }
            Self::Repository(message) => AppError::Repository(message),
            Self::Validation(message) => AppError::Validation(message),
            Self::Rejected(message) => AppError::DownloadSubmitRejected(message),
            Self::Ambiguous(message) => AppError::DownloadSubmitAmbiguous(message),
        }
    }
}

#[derive(Default, Clone)]
pub(super) struct StubDownloadClient {
    pub(super) queue_items: Arc<Mutex<Vec<DownloadQueueItem>>>,
    pub(super) history_items: Arc<Mutex<Vec<DownloadQueueItem>>>,
    pub(super) completed_downloads: Arc<Mutex<Vec<CompletedDownload>>>,
    pub(super) recent_completed_downloads: Arc<Mutex<Option<Vec<CompletedDownload>>>>,
    pub(super) deleted_items: Arc<Mutex<Vec<(String, bool)>>>,
    pub(super) deleted_requests: DeletedDownloadRequests,
    pub(super) delete_error: Arc<Mutex<Option<String>>>,
    pub(super) submit_error: Arc<Mutex<Option<StubSubmitError>>>,
    /// NZB payload the real pre-submission category gate is run against, so a
    /// caller-level test can exercise the production veto instead of a
    /// hand-written error string.
    pub(super) category_gate_nzb: Arc<Mutex<Option<Vec<u8>>>>,
    pub(super) grab_info_hash: Arc<Mutex<Option<String>>>,
    pub(super) submitted_release_titles: Arc<Mutex<Vec<String>>>,
    pub(super) submitted_source_passwords: Arc<Mutex<Vec<Option<String>>>>,
    /// Tracker-declared minimums as they reached the client, so a caller-level
    /// test can prove the clamp inputs survived the path under test.
    pub(super) submitted_seed_minimums: Arc<Mutex<Vec<crate::ReleaseSeedMinimums>>>,
    pub(super) queue_calls: Arc<Mutex<usize>>,
    pub(super) queue_for_title_calls: Arc<Mutex<Vec<String>>>,
    pub(super) history_calls: Arc<Mutex<usize>>,
    pub(super) recent_activity_calls: Arc<Mutex<Vec<usize>>>,
    pub(super) recent_activity_for_title_calls: Arc<Mutex<Vec<(String, usize)>>>,
    pub(super) completed_download_calls: Arc<Mutex<usize>>,
    pub(super) recent_completed_download_calls: Arc<Mutex<Vec<usize>>>,
    pub(super) targeted_completed_downloads: Arc<Mutex<HashMap<String, CompletedDownload>>>,
    pub(super) targeted_completed_download_calls: Arc<Mutex<Vec<String>>>,
    /// `(client_id, item_id)` for every pause the caller issued. Pause is how a
    /// `StopSeeding` seeding profile stops a finished torrent uploading, so the
    /// gate's non-removal actions are observable.
    pub(super) paused_requests: PausedDownloadRequests,
}

impl StubDownloadClient {
    pub(super) async fn set_delete_error(&self, error: Option<&str>) {
        *self.delete_error.lock().await = error.map(str::to_string);
    }

    pub(super) async fn set_submit_error(&self, error: Option<StubSubmitError>) {
        *self.submit_error.lock().await = error;
    }

    pub(super) async fn set_grab_info_hash(&self, info_hash: Option<&str>) {
        *self.grab_info_hash.lock().await = info_hash.map(str::to_string);
    }

    /// Serve `nzb` as the payload every submission would download, gated by the
    /// production pre-submission category check.
    pub(super) async fn set_category_gate_nzb(&self, nzb: Option<&[u8]>) {
        *self.category_gate_nzb.lock().await = nzb.map(<[u8]>::to_vec);
    }

    pub(super) async fn record_delete(
        &self,
        client_id: Option<&str>,
        client_type: Option<&str>,
        id: &str,
        is_history: bool,
        remove_data: bool,
    ) -> AppResult<()> {
        if let Some(error) = self.delete_error.lock().await.clone() {
            return Err(AppError::Repository(error));
        }
        self.deleted_items
            .lock()
            .await
            .push((id.to_string(), is_history));
        self.deleted_requests.lock().await.push((
            client_id.map(str::to_string),
            client_type.map(str::to_string),
            id.to_string(),
            is_history,
            remove_data,
        ));
        Ok(())
    }
}

#[async_trait]
impl DownloadClient for StubDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        // Mirrors production ordering: the NZB payload is inspected before the
        // client is ever handed the job, so a vetoed release leaves no trace.
        if let Some(nzb) = self.category_gate_nzb.lock().await.as_deref() {
            crate::enforce_nzb_category_gate(nzb, &request.title.facet)?;
        }
        let job_id = format!("job-for-{}", request.title.id);
        self.submitted_release_titles.lock().await.push(
            request
                .release_title
                .clone()
                .unwrap_or_else(|| request.title.name.clone()),
        );
        self.submitted_source_passwords
            .lock()
            .await
            .push(request.source_password.clone());
        self.submitted_seed_minimums
            .lock()
            .await
            .push(crate::ReleaseSeedMinimums {
                min_seed_ratio: request.tracker_min_seed_ratio,
                min_seed_time_minutes: request.tracker_min_seed_time_minutes,
                season_pack_seed_ratio: request.season_pack_seed_ratio,
                season_pack_seed_time_minutes: request.season_pack_seed_time_minutes,
            });
        if let Some(error) = self.submit_error.lock().await.clone() {
            return Err(error.into_app_error());
        }
        let mut queue_items = self.queue_items.lock().await;
        if !queue_items
            .iter()
            .any(|item| item.download_client_item_id == job_id)
        {
            let mut queued = queue_history_fixture_item(&job_id, DownloadQueueState::Queued, 0);
            queued.title_id = Some(request.title.id.clone());
            queued.title_name = request.title.name.clone();
            queued.facet = Some(request.title.facet.as_str().to_string());
            queue_items.push(queued);
        }
        Ok(DownloadGrabResult {
            job_id,
            client_id: None,
            client_type: "nzbget".to_string(),
            info_hash: self.grab_info_hash.lock().await.clone(),
        })
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        *self.queue_calls.lock().await += 1;
        Ok(self.queue_items.lock().await.clone())
    }

    async fn list_queue_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
        self.queue_for_title_calls
            .lock()
            .await
            .push(title_id.to_string());
        Ok(self.queue_items.lock().await.clone())
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        *self.history_calls.lock().await += 1;
        Ok(self.history_items.lock().await.clone())
    }

    async fn list_recent_activity(&self, limit: usize) -> AppResult<Vec<DownloadQueueItem>> {
        self.recent_activity_calls.lock().await.push(limit);
        Ok(self
            .history_items
            .lock()
            .await
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }

    async fn list_recent_activity_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.recent_activity_for_title_calls
            .lock()
            .await
            .push((title_id.to_string(), limit));
        Ok(self
            .history_items
            .lock()
            .await
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        *self.completed_download_calls.lock().await += 1;
        Ok(self.completed_downloads.lock().await.clone())
    }

    async fn list_recent_completed_downloads(
        &self,
        limit: usize,
    ) -> AppResult<Vec<CompletedDownload>> {
        self.recent_completed_download_calls
            .lock()
            .await
            .push(limit);
        let items = match self.recent_completed_downloads.lock().await.clone() {
            Some(items) => items,
            None => self.completed_downloads.lock().await.clone(),
        };
        Ok(items.into_iter().take(limit).collect())
    }

    async fn get_completed_download_for_source(
        &self,
        _client_id: &str,
        _client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<CompletedDownload>> {
        self.targeted_completed_download_calls
            .lock()
            .await
            .push(download_client_item_id.to_string());
        if let Some(found) = self
            .targeted_completed_downloads
            .lock()
            .await
            .get(download_client_item_id)
        {
            return Ok(Some(found.clone()));
        }
        let items = match self.recent_completed_downloads.lock().await.clone() {
            Some(items) => items,
            None => self.completed_downloads.lock().await.clone(),
        };
        Ok(items
            .into_iter()
            .find(|item| item.download_client_item_id == download_client_item_id))
    }

    async fn delete_queue_item(
        &self,
        id: &str,
        is_history: bool,
        remove_data: bool,
    ) -> AppResult<()> {
        self.record_delete(None, None, id, is_history, remove_data)
            .await
    }

    async fn delete_queue_item_for_client_id(
        &self,
        client_id: &str,
        id: &str,
        is_history: bool,
        remove_data: bool,
    ) -> AppResult<()> {
        self.record_delete(Some(client_id), None, id, is_history, remove_data)
            .await
    }

    async fn delete_queue_item_for_client(
        &self,
        client_type: &str,
        id: &str,
        is_history: bool,
        remove_data: bool,
    ) -> AppResult<()> {
        self.record_delete(None, Some(client_type), id, is_history, remove_data)
            .await
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        self.paused_requests
            .lock()
            .await
            .push((None, id.to_string()));
        Ok(())
    }

    async fn pause_queue_item_for_client(&self, client_id: &str, id: &str) -> AppResult<()> {
        self.paused_requests
            .lock()
            .await
            .push((Some(client_id.to_string()), id.to_string()));
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct TrackingDownloadQueueCommandRepo {
    pub(super) queued: Arc<Mutex<Vec<DownloadQueueCommandRecord>>>,
    pub(super) recovered_count: Arc<Mutex<u64>>,
}

impl TrackingDownloadQueueCommandRepo {
    pub(super) async fn seed_pending(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
    ) -> String {
        let id = format!("delete-command-{download_client_item_id}");
        self.queued.lock().await.push(DownloadQueueCommandRecord {
            id: id.clone(),
            action: scryer_domain::DownloadQueueCommandAction::Delete,
            client_id: client_id.map(str::to_string),
            client_type: client_type.to_string(),
            download_client_item_id: download_client_item_id.to_string(),
            is_history,
            status: scryer_domain::DownloadQueueDeleteStatus::Queued,
            error_text: None,
            requested_by_user_id: Some("admin".to_string()),
            started_at: None,
            finished_at: None,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        });
        id
    }

    pub(super) async fn get(&self, id: &str) -> Option<DownloadQueueCommandRecord> {
        self.queued
            .lock()
            .await
            .iter()
            .find(|record| record.id == id)
            .cloned()
    }
}

#[async_trait]
impl DownloadQueueCommandRepository for TrackingDownloadQueueCommandRepo {
    async fn queue_delete_command(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<DownloadQueueCommandRecord> {
        let id = self
            .seed_pending(client_id, client_type, download_client_item_id, is_history)
            .await;
        let mut queued = self.queued.lock().await;
        let record = queued
            .iter_mut()
            .find(|record| record.id == id)
            .expect("seeded queued delete command");
        record.requested_by_user_id = requested_by_user_id.map(str::to_string);
        Ok(record.clone())
    }

    async fn recover_stale_running_delete_commands(&self, _stale_seconds: i64) -> AppResult<u64> {
        Ok(*self.recovered_count.lock().await)
    }

    async fn list_pending_delete_commands(&self) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        Ok(self
            .queued
            .lock()
            .await
            .iter()
            .filter(|record| record.status == scryer_domain::DownloadQueueDeleteStatus::Queued)
            .cloned()
            .collect())
    }

    async fn mark_delete_command_running(&self, id: &str) -> AppResult<()> {
        let mut queued = self.queued.lock().await;
        let record = queued
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| AppError::NotFound(format!("queued delete {}", id)))?;
        record.status = scryer_domain::DownloadQueueDeleteStatus::Running;
        record.started_at = Some(Utc::now().to_rfc3339());
        record.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    async fn mark_delete_command_completed(&self, id: &str) -> AppResult<()> {
        let mut queued = self.queued.lock().await;
        let record = queued
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| AppError::NotFound(format!("queued delete {}", id)))?;
        record.status = scryer_domain::DownloadQueueDeleteStatus::Completed;
        record.finished_at = Some(Utc::now().to_rfc3339());
        record.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    async fn mark_delete_command_failed(
        &self,
        id: &str,
        error_text: Option<&str>,
    ) -> AppResult<()> {
        let mut queued = self.queued.lock().await;
        let record = queued
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| AppError::NotFound(format!("queued delete {}", id)))?;
        record.status = scryer_domain::DownloadQueueDeleteStatus::Failed;
        record.error_text = error_text.map(str::to_string);
        record.finished_at = Some(Utc::now().to_rfc3339());
        record.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    async fn list_latest_delete_commands_for_sources(
        &self,
        sources: &[(Option<String>, String, String, bool)],
    ) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        let queued = self.queued.lock().await;
        Ok(sources
            .iter()
            .filter_map(|(client_id, client_type, item_id, is_history)| {
                queued
                    .iter()
                    .find(|record| {
                        record.client_id.as_deref() == client_id.as_deref()
                            && record.client_type == *client_type
                            && record.download_client_item_id == *item_id
                            && record.is_history == *is_history
                    })
                    .cloned()
            })
            .collect())
    }

    async fn prune_terminal_delete_commands_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }
}
