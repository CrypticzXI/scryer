impl AppUseCase {
    pub async fn pause_download_queue_item(
        &self,
        actor: &User,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            None,
            download_client_item_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        if let Some(client_id) = client_id.filter(|value| !value.trim().is_empty()) {
            self.services
                .integrations
                .download_client
                .pause_queue_item_for_client(client_id, download_client_item_id)
                .await?;
        } else {
            self.services
                .integrations
                .download_client
                .pause_queue_item(download_client_item_id)
                .await?;
        }
        self.emit_download_queue_item_command_issued_event(
            actor,
            download_client_item_id.to_string(),
            scryer_domain::DownloadQueueCommandAction::Pause,
        )
        .await;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn resume_download_queue_item(
        &self,
        actor: &User,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            None,
            download_client_item_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        if let Some(client_id) = client_id.filter(|value| !value.trim().is_empty()) {
            self.services
                .integrations
                .download_client
                .resume_queue_item_for_client(client_id, download_client_item_id)
                .await?;
        } else {
            self.services
                .integrations
                .download_client
                .resume_queue_item(download_client_item_id)
                .await?;
        }
        self.emit_download_queue_item_command_issued_event(
            actor,
            download_client_item_id.to_string(),
            scryer_domain::DownloadQueueCommandAction::Resume,
        )
        .await;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn delete_download_queue_item(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
    ) -> AppResult<crate::DownloadQueueCommandRecord> {
        self.require_download_item_permission(
            actor,
            client_id,
            Some(client_type),
            download_client_item_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let client_type = self.normalize_download_client_type(client_type)?;
        let command = self
            .services
            .workflow
            .download_queue_commands
            .queue_delete_command(
                client_id,
                &client_type,
                download_client_item_id,
                is_history,
                Some(actor.id.as_str()),
            )
            .await?;
        self.emit_download_queue_item_command_issued_event(
            actor,
            download_client_item_id.to_string(),
            scryer_domain::DownloadQueueCommandAction::Delete,
        )
        .await;
        Ok(command)
    }
}
