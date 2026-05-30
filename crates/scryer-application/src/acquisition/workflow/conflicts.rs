impl AppUseCase {
    pub async fn trigger_title_wanted_search(
        &self,
        actor: &User,
        title_id: &str,
        conflict_policy: SubmissionConflictPolicy,
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

        let now = Utc::now();
        let outcome = if let Some(handler) = self.facet_registry.get(&title.facet) {
            if handler.has_episodes() {
                self.queue_monitored_series_items_for_search(&title, &now)
                    .await?
            } else if title.monitored {
                self.queue_monitored_movie_for_search(&title, &now, conflict_policy)
                    .await?
            } else {
                WantedSearchOutcome::default()
            }
        } else {
            WantedSearchOutcome::default()
        };

        if outcome.queued_count > 0 {
            self.runtime.acquisition.acquisition_wake.notify_one();
        }

        Ok(outcome)
    }
}
impl AppUseCase {
    pub async fn trigger_wanted_item_search(
        &self,
        actor: &User,
        wanted_item_id: &str,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<WantedSearchOutcome> {
        let item = self
            .services
            .workflow
            .wanted_items
            .get_wanted_item_by_id(wanted_item_id)
            .await?
            .ok_or_else(|| AppError::NotFound("wanted item not found".to_string()))?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&item.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound("title not found".to_string()))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if let Some(outcome) = self
            .handle_wanted_item_conflict(&title, &item, conflict_policy)
            .await?
        {
            return Ok(outcome);
        }

        let now = Utc::now();
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
        self.runtime.acquisition.acquisition_wake.notify_one();
        Ok(WantedSearchOutcome {
            queued_count: 1,
            skipped_in_progress_count: 0,
            conflict: None,
        })
    }
}
impl AppUseCase {
    async fn wanted_item_blocking_submissions(
        &self,
        title: &Title,
        item: &WantedItem,
    ) -> AppResult<Vec<SubmissionScopeConflict>> {
        let scope = self.wanted_item_submission_scope(item).await?;
        self.find_blocking_download_submissions(title, &scope).await
    }
}
impl AppUseCase {
    async fn handle_wanted_item_conflict(
        &self,
        title: &Title,
        item: &WantedItem,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<Option<WantedSearchOutcome>> {
        let conflicts = self.wanted_item_blocking_submissions(title, item).await?;
        if conflicts.is_empty() {
            return Ok(None);
        }

        match conflict_policy {
            SubmissionConflictPolicy::ReplaceEarly
                if conflicts.iter().all(|conflict| conflict.replaceable) =>
            {
                self.replace_blocking_download_submissions(&conflicts)
                    .await?;
                Ok(None)
            }
            SubmissionConflictPolicy::ReplaceEarly | SubmissionConflictPolicy::Abort => {
                let conflict = conflicts
                    .iter()
                    .find(|conflict| !conflict.replaceable)
                    .cloned()
                    .unwrap_or_else(|| conflicts[0].clone());
                Ok(Some(WantedSearchOutcome {
                    queued_count: 0,
                    skipped_in_progress_count: 0,
                    conflict: Some(conflict),
                }))
            }
            SubmissionConflictPolicy::Skip => Ok(Some(WantedSearchOutcome {
                queued_count: 0,
                skipped_in_progress_count: 1,
                conflict: Some(conflicts[0].clone()),
            })),
        }
    }
}
impl AppUseCase {
    async fn schedule_wanted_item_search_with_policy(
        &self,
        title: &Title,
        item: &WantedItem,
        next_search_at: &str,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<WantedSearchOutcome> {
        if let Some(outcome) = self
            .handle_wanted_item_conflict(title, item, conflict_policy)
            .await?
        {
            return Ok(outcome);
        }

        self.services
            .workflow
            .wanted_items
            .schedule_wanted_item_search(&WantedSearchTransition {
                id: item.id.clone(),
                next_search_at: Some(next_search_at.to_string()),
                last_search_at: item.last_search_at.clone(),
                search_count: item.search_count,
                current_score: item.current_score,
                grabbed_release: item.grabbed_release.clone(),
            })
            .await?;

        Ok(WantedSearchOutcome {
            queued_count: 1,
            skipped_in_progress_count: 0,
            conflict: None,
        })
    }
}
impl AppUseCase {
    async fn ensure_wanted_item_seeded_with_policy(
        &self,
        title: &Title,
        item: &WantedItem,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<WantedSearchOutcome> {
        if let Some(outcome) = self
            .handle_wanted_item_conflict(title, item, conflict_policy)
            .await?
        {
            return Ok(outcome);
        }

        self.services
            .workflow
            .wanted_items
            .ensure_wanted_item_seeded(item)
            .await?;

        Ok(WantedSearchOutcome {
            queued_count: 1,
            skipped_in_progress_count: 0,
            conflict: None,
        })
    }
}
impl AppUseCase {
    async fn queue_monitored_movie_for_search(
        &self,
        title: &Title,
        now: &DateTime<Utc>,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<WantedSearchOutcome> {
        let has_file = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .map(|files| !files.is_empty())
            .unwrap_or(false);

        if has_file {
            return Ok(WantedSearchOutcome::default());
        }

        let next_search_at = now.to_rfc3339();
        if let Some(item) = self
            .services
            .workflow
            .wanted_items
            .get_wanted_item_for_title(&title.id, None)
            .await?
        {
            if item.status == WantedStatus::Grabbed {
                return Ok(WantedSearchOutcome::default());
            }

            return self
                .schedule_wanted_item_search_with_policy(
                    title,
                    &item,
                    &next_search_at,
                    conflict_policy,
                )
                .await;
        }

        let baseline_date = title.first_aired.clone();
        let schedule = compute_search_schedule("movie", baseline_date.as_deref(), "primary", now);
        let item = WantedItem {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: None,
            title_slug: None,
            title_facet: None,
            library_id: Some(title.library_id.clone()),
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            search_phase: schedule.search_phase.to_string(),
            next_search_at: Some(next_search_at),
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

        self.ensure_wanted_item_seeded_with_policy(title, &item, conflict_policy)
            .await
    }
}
