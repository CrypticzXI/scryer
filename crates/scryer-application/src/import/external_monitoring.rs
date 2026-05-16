use super::*;

fn snapshot_payload_is_empty(payload: &ExternalImportMonitorSnapshotPayload) -> bool {
    match payload {
        ExternalImportMonitorSnapshotPayload::Movie { entries } => entries.is_empty(),
        ExternalImportMonitorSnapshotPayload::Series { entries } => entries.is_empty(),
    }
}

impl AppUseCase {
    pub async fn save_external_import_monitor_snapshot(
        &self,
        actor: &User,
        facet: MediaFacet,
        payload: ExternalImportMonitorSnapshotPayload,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        if snapshot_payload_is_empty(&payload) {
            return self
                .services
                .workflow
                .external_import_monitor_snapshots
                .delete_external_import_monitor_snapshot(&facet)
                .await;
        }

        self.services
            .workflow
            .external_import_monitor_snapshots
            .upsert_external_import_monitor_snapshot(&ExternalImportMonitorSnapshot {
                facet,
                payload,
                created_at: Utc::now().to_rfc3339(),
            })
            .await
    }

    pub async fn clear_external_import_monitor_snapshot(
        &self,
        actor: &User,
        facet: MediaFacet,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.services
            .workflow
            .external_import_monitor_snapshots
            .delete_external_import_monitor_snapshot(&facet)
            .await
    }

    pub(crate) async fn pending_external_import_monitor_snapshot(
        &self,
        facet: &MediaFacet,
    ) -> AppResult<Option<ExternalImportMonitorSnapshot>> {
        self.services
            .workflow
            .external_import_monitor_snapshots
            .get_external_import_monitor_snapshot(facet)
            .await
    }

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
        scope_kind: ExternalImportMonitorSnapshotChunkScopeKind,
        scope_key: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.services
            .workflow
            .external_import_monitor_snapshots
            .delete_external_import_monitor_snapshot_chunks(scope_kind, scope_key)
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
}
