use super::*;

impl AppUseCase {
    pub async fn append_external_import_monitor_snapshot_chunk(
        &self,
        actor: &User,
        chunk: ExternalImportMonitorSnapshotChunk,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.services
            .workflow
            .external_import_monitor_snapshots
            .append_external_import_monitor_snapshot_chunk(&chunk)
            .await
    }

    pub async fn clear_external_import_monitor_snapshot_chunks(
        &self,
        actor: &User,
        facet: MediaFacet,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.services
            .workflow
            .external_import_monitor_snapshots
            .delete_external_import_monitor_snapshot_chunks(
                crate::EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID,
                facet,
            )
            .await
    }

    pub async fn clear_external_import_monitor_snapshot_chunks_for_session(
        &self,
        actor: &User,
        session_id: &str,
        facet: MediaFacet,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.services
            .workflow
            .external_import_monitor_snapshots
            .delete_external_import_monitor_snapshot_chunks(session_id, facet)
            .await
    }

    pub async fn list_external_import_monitor_snapshot_chunks_for_session(
        &self,
        actor: &User,
        session_id: &str,
        facet: MediaFacet,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
        after_chunk_index: Option<i32>,
        limit: i32,
    ) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.services
            .workflow
            .external_import_monitor_snapshots
            .list_external_import_monitor_snapshot_chunk_batch(
                session_id,
                facet,
                entry_kind,
                after_chunk_index,
                limit,
            )
            .await
    }

    pub async fn clear_external_import_arr_source_session_chunks(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<()> {
        if session_id == crate::EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID {
            return Ok(());
        }
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            self.clear_external_import_monitor_snapshot_chunks_for_session(
                actor, session_id, facet,
            )
            .await?;
        }
        Ok(())
    }

    pub async fn clear_stale_external_import_arr_source_chunks_once(
        &self,
        actor: &User,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let mut cleanup_done = self
            .runtime
            .imports
            .external_import_source_chunk_cleanup_done
            .lock()
            .await;
        if *cleanup_done {
            return Ok(());
        }

        self.services
            .workflow
            .external_import_monitor_snapshots
            .delete_external_import_monitor_snapshot_chunks_except_session(
                crate::EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID,
            )
            .await?;
        *cleanup_done = true;
        Ok(())
    }

    pub async fn begin_external_import_monitor_warmup(
        &self,
        actor: &User,
        connection_fingerprint: &str,
    ) -> AppResult<ExternalImportMonitorWarmupBeginResult> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let initial_snapshot =
            ExternalImportMonitorWarmupProgressSnapshot::new(scryer_domain::Id::new().0);

        Ok(self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .begin(&actor.id, connection_fingerprint, initial_snapshot)
            .await)
    }

    pub async fn prune_terminal_external_import_warmup_sessions(
        &self,
        actor: &User,
        max_age: chrono::Duration,
    ) -> AppResult<Vec<String>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        Ok(self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .prune_terminal_older_than(max_age)
            .await)
    }

    pub async fn maintain_external_import_arr_source_sessions(
        &self,
        actor: &User,
    ) -> AppResult<()> {
        self.clear_stale_external_import_arr_source_chunks_once(actor)
            .await?;

        let terminal_ttl = chrono::Duration::hours(2);
        let removed_session_ids = self
            .prune_terminal_external_import_warmup_sessions(actor, terminal_ttl)
            .await?;
        for session_id in removed_session_ids {
            self.clear_external_import_arr_source_session_chunks(actor, &session_id)
                .await?;
        }

        Ok(())
    }

    pub async fn get_external_import_monitor_warmup_status(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<ExternalImportMonitorWarmupProgressSnapshot> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.runtime
            .imports
            .external_import_warmup_orchestrator
            .snapshot(&actor.id, session_id)
            .await
            .ok_or_else(|| AppError::NotFound(format!("no warmup session '{session_id}'")))
    }

    pub async fn subscribe_external_import_monitor_warmup_progress(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<tokio::sync::watch::Receiver<ExternalImportMonitorWarmupProgressSnapshot>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.runtime
            .imports
            .external_import_warmup_orchestrator
            .subscribe(&actor.id, session_id)
            .await
            .ok_or_else(|| AppError::NotFound(format!("no warmup session '{session_id}'")))
    }

    pub async fn cancel_external_import_monitor_warmup(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        Ok(self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .cancel(&actor.id, session_id)
            .await)
    }

    pub async fn remove_external_import_monitor_warmup_session(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        Ok(self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .remove(&actor.id, session_id)
            .await)
    }

    pub async fn claim_external_import_monitor_warmup(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<ExternalImportMonitorWarmupProgressSnapshot> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.runtime
            .imports
            .external_import_warmup_orchestrator
            .claim(&actor.id, session_id)
            .await
            .ok_or_else(|| AppError::NotFound(format!("no warmup session '{session_id}'")))
    }

    pub async fn external_import_monitor_warmup_connection_fingerprint(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<String> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.runtime
            .imports
            .external_import_warmup_orchestrator
            .connection_fingerprint(&actor.id, session_id)
            .await
            .ok_or_else(|| AppError::NotFound(format!("no warmup session '{session_id}'")))
    }

    pub async fn update_external_import_monitor_warmup_progress(
        &self,
        session_id: &str,
        mut snapshot: ExternalImportMonitorWarmupProgressSnapshot,
    ) {
        snapshot.touch();
        let _ = self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .update(session_id, snapshot)
            .await;
    }

    pub async fn set_external_import_monitor_warmup_scan_hints(
        &self,
        actor: &User,
        session_id: &str,
        scan_hints: LibraryScanHintSet,
    ) {
        let _ = self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .set_scan_hints(&actor.id, session_id, scan_hints)
            .await;
    }

    pub async fn set_external_import_arr_source_warmup_result(
        &self,
        session_id: &str,
        result: ExternalImportArrSourceWarmupResult,
    ) {
        let _ = self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .set_arr_source_result(session_id, result)
            .await;
    }

    pub async fn external_import_arr_source_warmup_result(
        &self,
        actor: &User,
        session_id: &str,
    ) -> AppResult<ExternalImportArrSourceWarmupResult> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.runtime
            .imports
            .external_import_warmup_orchestrator
            .arr_source_result(&actor.id, session_id)
            .await
            .ok_or_else(|| {
                AppError::NotFound(format!("no arr source warmup result '{session_id}'"))
            })
    }

    pub async fn external_import_monitor_warmup_scan_hints(
        &self,
        actor: &User,
        session_id: &str,
    ) -> Option<LibraryScanHintSet> {
        self.runtime
            .imports
            .external_import_warmup_orchestrator
            .scan_hints(&actor.id, session_id)
            .await
    }

    pub async fn acquire_external_import_apply_guard(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.runtime
            .imports
            .external_import_apply_lock
            .clone()
            .lock_owned()
            .await
    }
}
