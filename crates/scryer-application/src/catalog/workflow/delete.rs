#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TitleLogicalDeleteOptions {
    pub(crate) purge_recycle_bin_entries: bool,
    pub(crate) append_title_deleted_event: bool,
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
}
impl AppUseCase {
    pub(crate) async fn delete_title_logical_cleanup(
        &self,
        title: &scryer_domain::Title,
        actor_user_id: Option<String>,
        options: TitleLogicalDeleteOptions,
    ) -> AppResult<()> {
        self.purge_title_logical_dependents(title, options.purge_recycle_bin_entries)
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
        let (matching_movie_collection_ids, matching_interstitial_collection_ids) = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&media_file.title_id)
            .await?
            .into_iter()
            .filter(|collection| {
                collection.ordered_path.as_deref() == Some(media_file.file_path.as_str())
            })
            .fold((Vec::new(), Vec::new()), |mut acc, collection| {
                match collection.collection_type {
                    scryer_domain::CollectionType::Movie => acc.0.push(collection.id),
                    scryer_domain::CollectionType::Interstitial => acc.1.push(collection.id),
                    _ => {}
                }
                acc
            });

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
        for collection_id in matching_interstitial_collection_ids {
            if let Err(error) = self
                .services
                .catalog
                .shows
                .update_collection(
                    &collection_id,
                    CollectionUpdate {
                        clear_ordered_path: true,
                        ..Default::default()
                    },
                )
                .await
            {
                tracing::warn!(
                    error = %error,
                    file_id = %file_id,
                    collection_id = %collection_id,
                    file_path = %media_file.file_path,
                    "failed to clear matching interstitial collection ordered_path after media file delete"
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
