impl AppUseCase {
    async fn require_title_library_permission(
        &self,
        actor: &User,
        title_id: &str,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Title> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(actor, &title.library_id, permission)
            .await?;
        Ok(title)
    }
}
impl AppUseCase {
    async fn require_any_library_permission(
        &self,
        actor: &User,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<()> {
        if self
            .authorized_library_ids(actor, None, permission)
            .await?
            .is_empty()
        {
            Err(AppError::Unauthorized(
                "You do not have access to this library".to_string(),
            ))
        } else {
            Ok(())
        }
    }
}
impl AppUseCase {
    async fn filter_download_queue_items_for_permission(
        &self,
        actor: &User,
        items: Vec<DownloadQueueItem>,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, permission)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        if allowed_library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let can_view_operational_history = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let title_ids = items
            .iter()
            .filter_map(|item| item.title_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let title_library_cache = self
            .services
            .catalog
            .titles
            .get_by_ids(&title_ids)
            .await?
            .into_iter()
            .map(|title| (title.id, title.library_id))
            .collect::<HashMap<_, _>>();
        let mut visible = Vec::new();
        for item in items {
            let allowed = if let Some(title_id) = item.title_id.as_deref() {
                title_library_cache
                    .get(title_id)
                    .map(|library_id| allowed_library_ids.contains(library_id))
                    .unwrap_or(can_view_operational_history)
            } else {
                can_view_operational_history
            };
            if allowed {
                visible.push(item);
            }
        }
        Ok(visible)
    }
}
impl AppUseCase {
    async fn require_download_item_permission(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: Option<&str>,
        download_client_item_id: &str,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<()> {
        let item = self
            .find_download_queue_item_raw(client_id, client_type, download_client_item_id)
            .await?;
        if let Some(item) = item
            && let Some(title_id) = item.title_id.as_deref()
        {
            match self
                .require_title_library_permission(actor, title_id, permission)
                .await
            {
                Ok(_) => return Ok(()),
                Err(AppError::NotFound(_)) => {
                    tracing::warn!(
                        title_id,
                        download_client_item_id,
                        "download queue item references a missing title; using any-library permission"
                    );
                }
                Err(error) => return Err(error),
            }
        }
        self.require_any_library_permission(actor, permission).await
    }
}
impl AppUseCase {
    async fn require_completed_download_permission(
        &self,
        actor: &User,
        completed: &CompletedDownload,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<()> {
        if let Some(title_id) =
            crate::import_parameters::extract_parameter(&completed.parameters, "*scryer_title_id")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        {
            self.require_title_library_permission(actor, &title_id, permission)
                .await?;
            Ok(())
        } else {
            self.require_any_library_permission(actor, permission).await
        }
    }
}
impl AppUseCase {
    async fn collect_download_history_items_for_actor(
        &self,
        actor: &User,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let allowed_library_ids = self
            .authorized_library_ids(actor, None, permission)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let can_view_operational_history = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let (_, model) = self.current_download_queue_read_model().await?;
        let ordering = Self::legacy_download_queue_ordering(&model).await;
        Ok(ordering
            .iter()
            .map(|index| &model.items[*index])
            .filter(|item| is_history_download_state(&item.state))
            .filter(|item| {
                item.title_id.as_deref().map_or(
                    can_view_operational_history,
                    |title_id| {
                        model.title_library_ids.get(title_id).map_or(
                            can_view_operational_history,
                            |library_id| allowed_library_ids.contains(library_id),
                        )
                    },
                )
            })
            .cloned()
            .collect())
    }
}
