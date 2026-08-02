pub(crate) async fn has_enabled_download_clients(app: &AppUseCase) -> bool {
    app.services
        .integrations
        .download_client_configs
        .list(None)
        .await
        .map(|configs| configs.into_iter().any(|config| config.is_enabled))
        .unwrap_or(false)
}
impl AppUseCase {
    pub async fn get_wanted_item(&self, actor: &User, id: &str) -> AppResult<Option<AcquisitionScopeState>> {
        let Some(item) = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(id)
            .await?
        else {
            return Ok(None);
        };

        let library_id = match item.library_id.clone() {
            Some(library_id) => library_id,
            None => self
                .services
                .catalog
                .titles
                .get_by_id(&item.title_id)
                .await?
                .map(|title| title.library_id)
                .ok_or_else(|| AppError::NotFound(format!("title {}", item.title_id)))?,
        };
        self.require_library_permission(actor, &library_id, scryer_domain::LibraryPermission::View)
            .await?;
        Ok(Some(item))
    }

    /// Batch variant of [`Self::get_wanted_item`]: loads wanted items by id and
    /// silently drops those the actor cannot `View`.
    pub async fn get_wanted_items_by_ids(
        &self,
        actor: &User,
        ids: &[String],
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        let items = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states_by_ids(ids)
            .await?;
        self.filter_wanted_items_for_permission(
            actor,
            items,
            scryer_domain::LibraryPermission::View,
        )
        .await
    }
}
impl AppUseCase {
    pub async fn list_acquisition_scope_states(
        &self,
        actor: &User,
        query: AcquisitionScopeStatesQuery,
    ) -> AppResult<(Vec<AcquisitionScopeState>, i64)> {
        let requested_library_ids = query.library_ids.clone();
        let mut library_ids = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::View)
            .await?;
        if !requested_library_ids.is_empty() {
            let authorized = library_ids.into_iter().collect::<HashSet<_>>();
            library_ids = requested_library_ids
                .into_iter()
                .filter(|library_id| authorized.contains(library_id))
                .collect();
        }
        self.list_wanted_items_for_libraries(query, library_ids)
            .await
    }
}
impl AppUseCase {
    async fn list_wanted_items_for_libraries(
        &self,
        query: AcquisitionScopeStatesQuery,
        library_ids: Vec<String>,
    ) -> AppResult<(Vec<AcquisitionScopeState>, i64)> {
        let AcquisitionScopeStatesQuery {
            statuses,
            media_types,
            title_id,
            library_ids: _,
            title_search,
            latest_decision_codes,
            limit,
            offset,
        } = query;
        let title_search = title_search.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });
        if library_ids.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let items = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                statuses: statuses.clone(),
                media_types: media_types.clone(),
                title_id: title_id.clone(),
                library_ids: library_ids.clone(),
                title_search: title_search.clone(),
                latest_decision_codes: latest_decision_codes.clone(),
                limit,
                offset,
            })
            .await?;
        let total = self
            .services
            .workflow
            .acquisition_scope_states
            .count_acquisition_scope_states(AcquisitionScopeStatesQuery {
                statuses,
                media_types,
                title_id,
                library_ids,
                title_search,
                latest_decision_codes,
                ..AcquisitionScopeStatesQuery::default()
            })
            .await?;
        Ok((items, total))
    }
}
impl AppUseCase {
    /// Pause acquisition for a scope: user intent persisted on the state row.
    /// A paused scope is excluded from the derived target set until resumed. The
    /// identifier is a state-row id or a convergence scope key; a
    /// scope key with no row yet materializes one so the pause has somewhere to live.
    pub async fn pause_wanted_item(&self, actor: &User, wanted_item_id: &str) -> AppResult<()> {
        let item = self
            .resolve_or_create_wanted_state_row(wanted_item_id)
            .await?
            .ok_or_else(|| AppError::NotFound("wanted item not found".to_string()))?;
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

        self.services
            .workflow
            .acquisition_scope_states
            .transition_acquisition_scope_to_paused(&AcquisitionScopePauseTransition {
                id: item.id.clone(),
                last_search_at: item.last_search_at.clone(),
                current_score: item.current_score,
                grabbed_release: item.grabbed_release.clone(),
            })
            .await
    }
}
impl AppUseCase {
    /// Resume acquisition for a paused scope: it re-enters the derived target
    /// set. Existing coverage stays valid — the cursor only searches indexers
    /// it has not already searched under the current fingerprint. Accepts a
    /// state-row id or a convergence scope key.
    pub async fn resume_wanted_item(&self, actor: &User, wanted_item_id: &str) -> AppResult<()> {
        let item = self
            .resolve_or_create_wanted_state_row(wanted_item_id)
            .await?
            .ok_or_else(|| AppError::NotFound("wanted item not found".to_string()))?;
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

        self.services
            .workflow
            .acquisition_scope_states
            .update_acquisition_scope_status(
                &item.id,
                AcquisitionScopeStatus::Wanted.as_str(),
                item.last_search_at.as_deref(),
                item.current_score,
                item.grabbed_release.as_deref(),
            )
            .await?;
        self.runtime.acquisition.acquisition_wake.notify_one();
        Ok(())
    }
}
impl AppUseCase {
    async fn wanted_item_submission_scope(&self, item: &AcquisitionScopeState) -> AppResult<SubmissionScope> {
        let episode = if let Some(episode_id) = item.episode_id.as_deref() {
            self.services
                .catalog
                .shows
                .get_episode_by_id(episode_id)
                .await?
        } else {
            None
        };
        Ok(direct_download_submission_scope_for_wanted_item(
            item,
            episode.as_ref(),
        ))
    }
}
impl AppUseCase {
    pub(crate) async fn covered_wanted_item_ids_for_submission_scope(
        &self,
        title_id: &str,
        scope: &SubmissionScope,
        fallback_wanted_item_id: &str,
    ) -> AppResult<Vec<String>> {
        let items = self
            .services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                title_id: Some(title_id.to_string()),
                limit: 1000,
                ..AcquisitionScopeStatesQuery::default()
            })
            .await?;
        if items.is_empty() {
            return Ok(if fallback_wanted_item_id.is_empty() {
                Vec::new()
            } else {
                vec![fallback_wanted_item_id.to_string()]
            });
        }

        let episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(title_id)
            .await?;
        let fake_submission = DownloadSubmission {
            title_id: title_id.to_string(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: String::new(),
            download_client_id: None,
            download_client_type: String::new(),
            download_client_item_id: String::new(),
            source_hint: None,
            source_kind: None,
            source_title: None,
            request_signature: None,
            scope: scope.clone(),
        };

        let mut covered = items
            .iter()
            .filter(|item| {
                let episode_collection_id = item.episode_id.as_ref().and_then(|episode_id| {
                    episodes
                        .iter()
                        .find(|episode| &episode.id == episode_id)
                        .and_then(|episode| episode.collection_id.as_deref())
                });
                item.id == fallback_wanted_item_id
                    || submission_blocks_wanted_item(&fake_submission, item, episode_collection_id)
            })
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        covered.sort();
        covered.dedup();
        if covered.is_empty() && !fallback_wanted_item_id.is_empty() {
            covered.push(fallback_wanted_item_id.to_string());
        }
        Ok(covered)
    }
}
impl AppUseCase {
    /// Operator queue replacement: the blocking download is gone, so every
    /// scope it covered re-opens from scratch — score baseline cleared (the
    /// replacement decides on its own merits), coverage pruned, loop woken.
    pub(crate) async fn reset_wanted_items_for_submission_scope(
        &self,
        title_id: &str,
        scope: &SubmissionScope,
    ) -> AppResult<()> {
        let wanted_item_ids = self
            .covered_wanted_item_ids_for_submission_scope(title_id, scope, "")
            .await?;
        for wanted_item_id in wanted_item_ids {
            if let Some(item) = self
                .services
                .workflow
                .acquisition_scope_states
                .get_acquisition_scope_state_by_id(&wanted_item_id)
                .await?
            {
                self.services
                    .workflow
                    .acquisition_scope_states
                    .update_acquisition_scope_status(
                        &item.id,
                        AcquisitionScopeStatus::Wanted.as_str(),
                        None,
                        None,
                        None,
                    )
                    .await?;
                self.reopen_wanted_scope_for_acquisition(&item).await;
            }
        }
        Ok(())
    }
}
