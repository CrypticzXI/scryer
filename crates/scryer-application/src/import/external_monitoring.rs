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
        require(actor, &Entitlement::ManageConfig)?;

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
        require(actor, &Entitlement::ManageConfig)?;

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
}
