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
            .delete_external_import_monitor_snapshot_chunks(facet)
            .await
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
        session_id: &str,
        scan_hints: LibraryScanHintSet,
    ) {
        let _ = self
            .runtime
            .imports
            .external_import_warmup_orchestrator
            .set_scan_hints(session_id, scan_hints)
            .await;
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
}
