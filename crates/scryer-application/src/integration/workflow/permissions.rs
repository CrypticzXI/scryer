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
        let mut title_library_cache = HashMap::<String, Option<String>>::new();
        let mut visible = Vec::new();
        for item in items {
            let allowed = if let Some(title_id) = item.title_id.as_deref() {
                let library_id = if let Some(cached) = title_library_cache.get(title_id) {
                    cached.clone()
                } else {
                    let library_id = self
                        .services
                        .catalog
                        .titles
                        .get_by_id(title_id)
                        .await?
                        .map(|title| title.library_id);
                    title_library_cache.insert(title_id.to_string(), library_id.clone());
                    library_id
                };
                library_id
                    .as_ref()
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
            self.require_title_library_permission(actor, title_id, permission)
                .await?;
            return Ok(());
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
        let items = self.collect_download_history_items(true).await?;
        self.filter_download_queue_items_for_permission(actor, items, permission)
            .await
    }
}
impl AppUseCase {
    async fn collect_download_snapshot_items_for_actor(
        &self,
        actor: &User,
        permission: scryer_domain::LibraryPermission,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let items = self
            .collect_download_snapshot_items(true, true, true)
            .await?;
        self.filter_download_queue_items_for_permission(actor, items, permission)
            .await
    }
}
