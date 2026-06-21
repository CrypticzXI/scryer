use super::*;

pub(super) type DeleteOperationLog = Arc<Mutex<Vec<String>>>;
pub(super) type OptionalDeleteOperationLog = Arc<Mutex<Option<DeleteOperationLog>>>;
pub(super) type TrackedDownloadStateKey = (String, String, String);
pub(super) type TrackedDownloadStates = Arc<Mutex<HashMap<TrackedDownloadStateKey, String>>>;
pub(super) type DownloadSubmissionIdentities =
    Arc<Mutex<HashMap<TrackedDownloadStateKey, DownloadSubmissionIdentity>>>;
pub(super) type DownloadIdentityStates = Arc<Mutex<HashMap<String, String>>>;
pub(super) type ImportIdentities = Arc<Mutex<HashMap<String, DownloadSubmissionIdentity>>>;
pub(super) type DeletedDownloadRequest = (Option<String>, Option<String>, String, bool);
pub(super) type DeletedDownloadRequests = Arc<Mutex<Vec<DeletedDownloadRequest>>>;

#[derive(Default)]
pub(super) struct MockTitleRepo {
    pub(super) store: Arc<Mutex<Vec<Title>>>,
    pub(super) create_or_get_existing_error: Arc<Mutex<Option<String>>>,
    pub(super) delete_operation_log: OptionalDeleteOperationLog,
}
#[derive(Default)]
pub(super) struct RecordingJobRunRepo {
    pub(super) runs: Arc<Mutex<Vec<JobRunRecord>>>,
}

impl RecordingJobRunRepo {
    pub(super) async fn seed(&self, run: JobRunRecord) {
        self.runs.lock().await.push(run);
    }
}

#[async_trait]
impl JobRunRepository for RecordingJobRunRepo {
    async fn create_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        let mut runs = self.runs.lock().await;
        runs.push(run.clone());
        Ok(run.clone())
    }

    async fn update_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        let mut runs = self.runs.lock().await;
        if let Some(existing) = runs.iter_mut().find(|candidate| candidate.id == run.id) {
            *existing = run.clone();
        } else {
            runs.push(run.clone());
        }
        Ok(run.clone())
    }

    async fn get_job_run(&self, run_id: &str) -> AppResult<Option<JobRunRecord>> {
        Ok(self
            .runs
            .lock()
            .await
            .iter()
            .find(|run| run.id == run_id)
            .cloned())
    }

    async fn list_job_runs(
        &self,
        job_key: Option<JobKey>,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        Ok(sorted_limited_job_runs(
            self.runs
                .lock()
                .await
                .iter()
                .filter(|run| job_key.is_none_or(|job_key| run.job_key == job_key))
                .cloned()
                .collect(),
            limit,
        ))
    }

    async fn list_job_runs_for_actor(
        &self,
        job_key: Option<JobKey>,
        actor_user_id: &str,
        limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        Ok(sorted_limited_job_runs(
            self.runs
                .lock()
                .await
                .iter()
                .filter(|run| job_key.is_none_or(|job_key| run.job_key == job_key))
                .filter(|run| run.actor_user_id.as_deref() == Some(actor_user_id))
                .cloned()
                .collect(),
            limit,
        ))
    }

    async fn list_active_job_runs(&self) -> AppResult<Vec<JobRunRecord>> {
        Ok(self
            .runs
            .lock()
            .await
            .iter()
            .filter(|run| !run.status.is_terminal())
            .cloned()
            .collect())
    }
}

pub(super) fn sorted_limited_job_runs(
    mut runs: Vec<JobRunRecord>,
    limit: usize,
) -> Vec<JobRunRecord> {
    runs.sort_by(|left, right| {
        right
            .started_at
            .cmp(&left.started_at)
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| right.id.cmp(&left.id))
    });
    runs.truncate(limit);
    runs
}

impl MockTitleRepo {
    pub(super) async fn fail_create_or_get_existing(&self, message: &str) {
        *self.create_or_get_existing_error.lock().await = Some(message.to_string());
    }

    pub(super) async fn set_delete_operation_log(&self, operation_log: Arc<Mutex<Vec<String>>>) {
        *self.delete_operation_log.lock().await = Some(operation_log);
    }
}
#[derive(Default)]
pub(super) struct BlockingTitleImageRepo {
    pub(super) clear_calls: AtomicUsize,
    pub(super) release_clear: Notify,
}

#[async_trait]
impl TitleImageRepository for BlockingTitleImageRepo {
    async fn list_title_image_refresh_work(
        &self,
        _limit: usize,
        _skipped: &[TitleImageSyncTask],
    ) -> AppResult<Vec<TitleImageSyncTask>> {
        Ok(Vec::new())
    }

    async fn clear_title_image_cache(&self) -> AppResult<()> {
        self.clear_calls.fetch_add(1, Ordering::SeqCst);
        self.release_clear.notified().await;
        Ok(())
    }

    async fn upsert_title_image_source_result(
        &self,
        _title_id: &str,
        _result: TitleImageSourceResult,
        _event: Option<NewDomainEvent>,
    ) -> AppResult<Option<DomainEvent>> {
        Ok(None)
    }

    async fn get_title_image_blob(
        &self,
        _title_id: &str,
        _kind: TitleImageKind,
        _variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>> {
        Ok(None)
    }
}

#[async_trait]
impl TitleRepository for MockTitleRepo {
    async fn list(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        let list = self.store.lock().await.clone();
        let normalized_query = query.map(|value| value.to_lowercase());
        Ok(list
            .into_iter()
            .filter(|title| {
                let facet_match = facet
                    .as_ref()
                    .is_none_or(|expected| &title.facet == expected);
                let query_match = normalized_query
                    .as_ref()
                    .is_none_or(|term| title.name.to_lowercase().contains(term));
                facet_match && query_match
            })
            .collect())
    }

    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>> {
        let requested: Vec<&str> = values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect();
        let list = self.store.lock().await;
        let mut matches = Vec::new();
        let mut seen = HashSet::new();
        for value in requested {
            if let Some(title) = list.iter().find(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case(source) && external_id.value == value
                })
            }) && seen.insert(title.id.clone())
            {
                matches.push(title.clone());
            }
        }
        Ok(matches)
    }

    async fn list_for_matching(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list(facet, query).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
        let list = self.store.lock().await;
        Ok(list.iter().find(|title| title.id == id).cloned())
    }

    async fn get_by_facet_and_slug(
        &self,
        facet: MediaFacet,
        slug: &str,
    ) -> AppResult<Option<Title>> {
        let normalized_slug = slug.trim();
        if normalized_slug.is_empty() {
            return Ok(None);
        }

        let list = self.store.lock().await;
        let matches = list
            .iter()
            .filter(|title| {
                title.facet == facet
                    && title.slug.as_deref().is_some_and(|candidate| {
                        candidate.trim().eq_ignore_ascii_case(normalized_slug)
                    })
            })
            .cloned()
            .collect::<Vec<_>>();

        match matches.as_slice() {
            [] => Ok(None),
            [title] => Ok(Some(title.clone())),
            _ => Err(AppError::Validation(
                "multiple titles found for slug lookup".into(),
            )),
        }
    }

    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>> {
        let list = self.store.lock().await;
        Ok(list
            .iter()
            .find(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case(source) && external_id.value == value
                })
            })
            .cloned())
    }

    async fn find_by_external_id_in_facet(
        &self,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>> {
        let list = self.store.lock().await;
        Ok(list
            .iter()
            .find(|title| {
                title.facet == facet
                    && title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case(source)
                            && external_id.value == value
                    })
            })
            .cloned())
    }

    async fn create_or_get_existing(&self, title: Title) -> AppResult<CreateTitleOutcome> {
        if let Some(message) = self.create_or_get_existing_error.lock().await.clone() {
            return Err(AppError::Repository(message));
        }

        let mut list = self.store.lock().await;
        let mut matching_ids = list
            .iter()
            .filter(|existing| {
                existing.facet == title.facet
                    && existing.external_ids.iter().any(|existing_external_id| {
                        title.external_ids.iter().any(|incoming_external_id| {
                            existing_external_id
                                .source
                                .eq_ignore_ascii_case(&incoming_external_id.source)
                                && existing_external_id.value == incoming_external_id.value
                        })
                    })
            })
            .map(|existing| existing.id.clone())
            .collect::<Vec<_>>();
        matching_ids.sort();
        matching_ids.dedup();

        if matching_ids.len() > 1 {
            return Err(AppError::Validation(
                "external ids already map to multiple titles".into(),
            ));
        }

        if let Some(existing_id) = matching_ids.first()
            && let Some(existing) = list.iter().find(|entry| entry.id == *existing_id)
        {
            return Ok(CreateTitleOutcome {
                title: existing.clone(),
                reused_existing: true,
            });
        }

        list.push(title.clone());
        Ok(CreateTitleOutcome {
            title,
            reused_existing: false,
        })
    }

    async fn create(&self, title: Title) -> AppResult<Title> {
        self.store.lock().await.push(title.clone());
        Ok(title)
    }

    async fn list_titles_due_for_hydration(
        &self,
        _limit: usize,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<PendingTitleHydration>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|title| {
                title.metadata_fetched_at.is_none()
                    && !excluded_facets.iter().any(|facet| facet == &title.facet)
                    && title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case("tvdb")
                            && !external_id.value.trim().is_empty()
                    })
            })
            .cloned()
            .map(|title| PendingTitleHydration {
                title,
                attempt_count: 0,
            })
            .collect())
    }

    async fn list_anime_title_ids_missing_anibridge_scoped_external_ids(
        &self,
        _limit: usize,
    ) -> AppResult<Vec<String>> {
        Ok(vec![])
    }

    async fn mark_title_metadata_hydration_due_now(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn schedule_title_metadata_hydration_retry(
        &self,
        _: &str,
        _: &str,
        _: i64,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn clear_title_metadata_hydration_retry_state(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn update_metadata(
        &self,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
        root_folder_id: Option<String>,
    ) -> AppResult<Title> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;

        if let Some(name) = name {
            let normalized = name.trim();
            if normalized.is_empty() {
                return Err(AppError::Validation("title name cannot be empty".into()));
            }
            title.name = normalized.to_string();
        }

        if let Some(facet) = facet {
            if facet != title.facet {
                return Err(AppError::Validation(
                    "changing a title facet is not supported because titles cannot move between libraries"
                        .into(),
                ));
            }
            title.facet = facet;
        }

        if let Some(tags) = tags {
            title.tags = normalize_tags(&tags);
        }

        if let Some(root_folder_id) = root_folder_id {
            title.root_folder_id = root_folder_id;
        }

        Ok(title.clone())
    }

    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.monitored = monitored;
        Ok(title.clone())
    }

    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.name = metadata.name.unwrap_or(title.name.clone());
        title.year = metadata.year;
        title.overview = metadata.overview;
        title.poster_url = metadata.poster_url;
        title.background_url = metadata.background_url;
        title.sort_title = metadata.sort_title;
        title.slug = metadata.slug;
        title.imdb_id = metadata.imdb_id;
        title.runtime_minutes = metadata.runtime_minutes;
        title.genres = metadata.genres;
        title.content_status = metadata.content_status;
        title.language = metadata.language;
        title.first_aired = metadata.first_aired;
        title.network = metadata.network;
        title.studio = metadata.studio;
        title.country = metadata.country;
        title.aliases = metadata.aliases;
        title.tagged_aliases = metadata.tagged_aliases;
        title.metadata_language = metadata.metadata_language;
        title.metadata_fetched_at = Some(chrono::Utc::now());
        Ok(title.clone())
    }

    async fn replace_match_state(
        &self,
        id: &str,
        external_ids: Vec<ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.external_ids = external_ids;
        title.tags = tags;
        Ok(title.clone())
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        if let Some(operation_log) = self.delete_operation_log.lock().await.clone() {
            operation_log
                .lock()
                .await
                .push(format!("delete_title:{id}"));
        }
        let mut list = self.store.lock().await;
        let position = list
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        list.remove(position);
        Ok(())
    }

    async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.folder_path = Some(folder_path.to_string());
        Ok(())
    }

    async fn clear_folder_path(&self, id: &str) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.folder_path = None;
        Ok(())
    }

    async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
        let mut list = self.store.lock().await;
        let mut count = 0u64;
        for title in list.iter_mut() {
            if title.metadata_language.is_some() {
                title.metadata_language = None;
                count += 1;
            }
        }
        Ok(count)
    }
}
