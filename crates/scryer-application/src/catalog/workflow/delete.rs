#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TitleLogicalDeleteOptions {
    pub(crate) purge_recycle_bin_entries: bool,
    pub(crate) append_title_deleted_event: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteTitlesJobItem {
    pub title_id: String,
    pub preview_fingerprint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteTitlesJobRequest {
    pub items: Vec<DeleteTitlesJobItem>,
    pub delete_files_on_disk: bool,
    pub typed_confirmation: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DeleteTitlesJobAccepted {
    pub job_run: JobRun,
    pub accepted_title_ids: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TitleDeletionProgress {
    status: String,
    phase: String,
    total: usize,
    processed: usize,
    succeeded: usize,
    failed: usize,
    current_title: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TitleDeletionFailure {
    title_id: String,
    error: String,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TitleDeletionSummary {
    total: usize,
    succeeded: usize,
    failed: usize,
    succeeded_title_ids: Vec<String>,
    failures: Vec<TitleDeletionFailure>,
}
impl AppUseCase {
    pub(crate) async fn should_remove_completed_download(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
        client_id: &str,
    ) -> bool {
        match self
            .read_download_client_routing_entry(library_id, facet, client_id)
            .await
            .ok()
            .flatten()
        {
            Some(entry) => entry.remove_completed,
            None => default_download_client_routing_entry().remove_completed,
        }
    }
}
impl AppUseCase {
    pub(crate) async fn should_remove_failed_download(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
        client_id: &str,
    ) -> bool {
        match self
            .read_download_client_routing_entry(library_id, facet, client_id)
            .await
            .ok()
            .flatten()
        {
            Some(entry) => entry.remove_failed,
            None => default_download_client_routing_entry().remove_failed,
        }
    }
}
impl AppUseCase {
    pub async fn delete_title(
        &self,
        actor: &User,
        id: &str,
        delete_files_on_disk: bool,
        delete_confirmation: Option<DeleteExecutionConfirmation>,
    ) -> AppResult<()> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if delete_files_on_disk {
            let delete_confirmation = delete_confirmation.ok_or_else(|| {
                AppError::Validation(
                    "delete preview confirmation is required before deleting files on disk".into(),
                )
            })?;
            let DeleteExecutionConfirmation {
                preview_fingerprint,
                typed_confirmation,
            } = delete_confirmation;
            self.execute_delete_title_files(
                id,
                &preview_fingerprint,
                typed_confirmation.as_deref(),
            )
            .await?;
        }

        self.delete_title_logical_cleanup(
            &title,
            Some(actor.id.clone()),
            TitleLogicalDeleteOptions {
                purge_recycle_bin_entries: true,
                append_title_deleted_event: true,
            },
        )
        .await?;

        Ok(())
    }

    pub async fn start_delete_titles_job(
        &self,
        actor: &User,
        request: DeleteTitlesJobRequest,
    ) -> AppResult<DeleteTitlesJobAccepted> {
        if request.items.is_empty() {
            return Err(AppError::Validation(
                "at least one title is required for deletion".into(),
            ));
        }
        let deletion_guard = self
            .runtime
            .jobs
            .title_deletion_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| AppError::Validation("a title deletion job is already running".into()))?;
        if self
            .runtime
            .jobs
            .job_run_tracker
            .has_active_job(JobKey::TitleDeletion)
            .await
        {
            return Err(AppError::Validation(
                "a title deletion job is already running".into(),
            ));
        }

        let mut seen = HashSet::new();
        for item in &request.items {
            if item.title_id.trim().is_empty() {
                return Err(AppError::Validation("title id is required".into()));
            }
            if !seen.insert(item.title_id.clone()) {
                return Err(AppError::Validation(format!(
                    "duplicate title id in delete request: {}",
                    item.title_id
                )));
            }
            if request.delete_files_on_disk
                && item
                    .preview_fingerprint
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
            {
                return Err(AppError::Validation(format!(
                    "delete preview confirmation is required before deleting files on disk for title {}",
                    item.title_id
                )));
            }
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(&item.title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {}", item.title_id)))?;
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
        }

        let accepted_title_ids = request
            .items
            .iter()
            .map(|item| item.title_id.clone())
            .collect::<Vec<_>>();
        let now = chrono::Utc::now();
        let mut run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::TitleDeletion,
            operation_type: format!("title_deletion:{}", accepted_title_ids.len()),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: serde_json::to_string(&TitleDeletionProgress {
                status: JobRunStatus::Running.as_str().to_string(),
                phase: "queued".to_string(),
                total: accepted_title_ids.len(),
                processed: 0,
                succeeded: 0,
                failed: 0,
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
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
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
        let actor_user_id = actor.id.clone();
        tokio::spawn(async move {
            app.run_delete_titles_job(run, actor_user_id, request, deletion_guard)
                .await;
        });

        Ok(DeleteTitlesJobAccepted {
            job_run: run_payload,
            accepted_title_ids,
        })
    }
}
impl AppUseCase {
    async fn run_delete_titles_job(
        &self,
        mut run: JobRunRecord,
        actor_user_id: String,
        request: DeleteTitlesJobRequest,
        _deletion_guard: tokio::sync::OwnedMutexGuard<()>,
    ) {
        let total = request.items.len();
        let mut succeeded_title_ids = Vec::new();
        let mut failures = Vec::new();

        for item in request.items {
            let phase = if request.delete_files_on_disk {
                "deleting_files"
            } else {
                "cleaning_catalog"
            };
            let _ = self
                .update_title_deletion_progress(
                    &mut run,
                    TitleDeletionProgress {
                        status: JobRunStatus::Running.as_str().to_string(),
                        phase: phase.to_string(),
                        total,
                        processed: succeeded_title_ids.len() + failures.len(),
                        succeeded: succeeded_title_ids.len(),
                        failed: failures.len(),
                        current_title: Some(item.title_id.clone()),
                    },
                )
                .await;

            let result = self
                .delete_title_job_item(&actor_user_id, &item, request.delete_files_on_disk, request.typed_confirmation.as_deref())
                .await;
            match result {
                Ok(()) => succeeded_title_ids.push(item.title_id),
                Err(error) => failures.push(TitleDeletionFailure {
                    title_id: item.title_id,
                    error: error.to_string(),
                }),
            }

            let _ = self
                .update_title_deletion_progress(
                    &mut run,
                    TitleDeletionProgress {
                        status: JobRunStatus::Running.as_str().to_string(),
                        phase: "running".to_string(),
                        total,
                        processed: succeeded_title_ids.len() + failures.len(),
                        succeeded: succeeded_title_ids.len(),
                        failed: failures.len(),
                        current_title: None,
                    },
                )
                .await;
        }

        let summary = TitleDeletionSummary {
            total,
            succeeded: succeeded_title_ids.len(),
            failed: failures.len(),
            succeeded_title_ids,
            failures,
        };
        let status = if summary.failed == 0 {
            JobRunStatus::Completed
        } else if summary.succeeded == 0 {
            JobRunStatus::Failed
        } else {
            JobRunStatus::Warning
        };
        let summary_text = match status {
            JobRunStatus::Completed => format!("Deleted {} title(s)", summary.succeeded),
            JobRunStatus::Warning => format!(
                "Deleted {} title(s); {} failed",
                summary.succeeded, summary.failed
            ),
            JobRunStatus::Failed => format!("Failed to delete {} title(s)", summary.failed),
            _ => "Title deletion finished".to_string(),
        };
        if let Err(error) = self
            .finish_title_deletion_job(run, status, summary_text, summary)
            .await
        {
            warn!(error = %error, "failed to finish title deletion job");
        }
    }

    async fn delete_title_job_item(
        &self,
        actor_user_id: &str,
        item: &DeleteTitlesJobItem,
        delete_files_on_disk: bool,
        typed_confirmation: Option<&str>,
    ) -> AppResult<()> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&item.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", item.title_id)))?;

        if delete_files_on_disk {
            let preview_fingerprint = item
                .preview_fingerprint
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    AppError::Validation(
                        "delete preview confirmation is required before deleting files on disk"
                            .into(),
                    )
                })?;
            self.execute_delete_title_files(
                &item.title_id,
                preview_fingerprint,
                typed_confirmation,
            )
            .await?;
        }

        self.delete_title_logical_cleanup(
            &title,
            Some(actor_user_id.to_string()),
            TitleLogicalDeleteOptions {
                purge_recycle_bin_entries: true,
                append_title_deleted_event: true,
            },
        )
        .await
    }

    async fn update_title_deletion_progress(
        &self,
        run: &mut JobRunRecord,
        progress: TitleDeletionProgress,
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

    async fn finish_title_deletion_job(
        &self,
        mut run: JobRunRecord,
        status: JobRunStatus,
        summary_text: String,
        summary: TitleDeletionSummary,
    ) -> AppResult<()> {
        let completed_at = chrono::Utc::now();
        run.status = status;
        run.progress_json = Some(
            serde_json::json!({
                "status": status.as_str(),
                "phase": "completed",
                "total": summary.total,
                "processed": summary.total,
                "succeeded": summary.succeeded,
                "failed": summary.failed,
                "currentTitle": null,
            })
            .to_string(),
        );
        run.summary_text = Some(summary_text);
        run.summary_json = serde_json::to_string(&summary).ok();
        run.error_text = (status == JobRunStatus::Failed)
            .then(|| "all title deletions failed".to_string());
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        let updated = self.services.events.job_runs.update_job_run(&run).await?;
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
                updated.actor_user_id.clone(),
                updated.id.clone(),
                payload,
            ))
            .await;
        Ok(())
    }

    pub(crate) async fn delete_title_logical_cleanup(
        &self,
        title: &scryer_domain::Title,
        actor_user_id: Option<String>,
        options: TitleLogicalDeleteOptions,
    ) -> AppResult<()> {
        self.purge_title_logical_dependents(
            title,
            options.purge_recycle_bin_entries,
            actor_user_id.as_deref(),
        )
        .await?;
        self.delete_title_row(title, actor_user_id, options.append_title_deleted_event)
            .await
    }
}
impl AppUseCase {
    pub(crate) async fn delete_title_row(
        &self,
        title: &scryer_domain::Title,
        actor_user_id: Option<String>,
        append_title_deleted_event: bool,
    ) -> AppResult<()> {
        let title_id = title.id.as_str();

        self.services.catalog.titles.delete(title_id).await?;

        if append_title_deleted_event {
            let _ = self
                .append_domain_event(new_title_domain_event(
                    actor_user_id,
                    title,
                    DomainEventPayload::TitleDeleted(TitleDeletedEventData {
                        title: title_context_snapshot(title),
                    }),
                ))
                .await;
        }

        Ok(())
    }
}
impl AppUseCase {
    pub async fn delete_media_file(
        &self,
        actor: &User,
        file_id: &str,
        delete_from_disk: bool,
        delete_confirmation: Option<DeleteExecutionConfirmation>,
    ) -> AppResult<()> {
        let media_file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        let library_id = self
            .services
            .catalog
            .libraries
            .title_library_id(&media_file.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", media_file.title_id)))?;
        self.require_library_permission(
            actor,
            &library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let matching_movie_collection_ids = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&media_file.title_id)
            .await?
            .into_iter()
            .filter(|collection| {
                collection.ordered_path.as_deref() == Some(media_file.file_path.as_str())
            })
            .filter_map(|collection| {
                if collection.collection_type == scryer_domain::CollectionType::Movie {
                    Some(collection.id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        if delete_from_disk {
            let delete_confirmation = delete_confirmation.ok_or_else(|| {
                AppError::Validation(
                    "delete preview confirmation is required before deleting files on disk".into(),
                )
            })?;
            let DeleteExecutionConfirmation {
                preview_fingerprint,
                typed_confirmation,
            } = delete_confirmation;
            self.execute_delete_media_file(
                file_id,
                &preview_fingerprint,
                typed_confirmation.as_deref(),
            )
            .await?;
        }

        self.services
            .library
            .media_files
            .delete_media_file(file_id)
            .await?;
        for collection_id in matching_movie_collection_ids {
            if let Err(error) = self
                .services
                .catalog
                .shows
                .delete_collection(&collection_id)
                .await
            {
                tracing::warn!(
                    error = %error,
                    file_id = %file_id,
                    collection_id = %collection_id,
                    file_path = %media_file.file_path,
                    "failed to delete matching movie collection after media file delete"
                );
            }
        }
        info!(
            file_id = %file_id,
            file_path = %media_file.file_path,
            delete_from_disk = %delete_from_disk,
            "media file deleted"
        );

        if delete_from_disk
            && let Ok(Some(title)) = self
                .services
                .catalog
                .titles
                .get_by_id(&media_file.title_id)
                .await
        {
            let _ = self
                .append_domain_event(new_title_domain_event(
                    Some(actor.id.clone()),
                    &title,
                    DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                        title: title_context_snapshot(&title),
                        media_updates: vec![deleted_media_update(media_file.file_path.clone())],
                        file_id: Some(media_file.id.clone()),
                        reason: MediaFileDeletedReason::Deleted,
                        episode_ids: media_file.episode_id.iter().cloned().collect(),
                    }),
                ))
                .await;
        }

        Ok(())
    }
}
impl AppUseCase {
    pub async fn delete_collection(&self, actor: &User, collection_id: &str) -> AppResult<()> {
        self.require_collection_permission(
            actor,
            collection_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        self.services
            .catalog
            .shows
            .delete_collection(collection_id)
            .await?;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn delete_episode(&self, actor: &User, episode_id: &str) -> AppResult<()> {
        self.require_episode_permission(
            actor,
            episode_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        self.services
            .catalog
            .shows
            .delete_episode(episode_id)
            .await?;
        Ok(())
    }
}
