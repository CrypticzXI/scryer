use super::*;

use async_trait::async_trait;
use scryer_application::{
    AppResult, ExternalImportMonitorSnapshotChunk, ExternalImportMonitorSnapshotEntryKind,
    ExternalImportMonitorSnapshotRepository,
};
use scryer_domain::MediaFacet;

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};

#[derive(Clone)]
pub struct ExternalImportMonitorStore {
    datastore: StoreDatastore,
}

impl ExternalImportMonitorStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl ExternalImportMonitorSnapshotRepository for ExternalImportMonitorStore {
    async fn append_external_import_monitor_snapshot_chunk(
        &self,
        chunk: &ExternalImportMonitorSnapshotChunk,
    ) -> AppResult<()> {
        let chunk = chunk.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "append_external_import_monitor_snapshot_chunk",
            move |tx| {
                let chunk = chunk.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO external_import_monitor_snapshot_chunks
                         (facet, entry_kind, chunk_index, payload_ndjson, created_at)
                         VALUES ({}, {}, {}, {}, {})
                         ON CONFLICT(facet, entry_kind, chunk_index) DO UPDATE SET
                             payload_ndjson = excluded.payload_ndjson,
                             created_at = excluded.created_at",
                        &[
                            SqlArg::Text(chunk.facet.as_str().to_string()),
                            SqlArg::Text(chunk.entry_kind.as_str().to_string()),
                            SqlArg::I32(chunk.chunk_index),
                            SqlArg::Text(chunk.payload_ndjson),
                            SqlArg::Timestamp(parse_datetime_or_now(Some(&chunk.created_at))),
                        ],
                    )
                    .await
                    .map_err(map_snapshot_chunk_error)?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn list_external_import_monitor_snapshot_chunk_batch(
        &self,
        facet: MediaFacet,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
        after_chunk_index: Option<i32>,
        limit: i32,
    ) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>> {
        fetch_snapshot_chunks(
            self.datastore.read_exec(),
            "SELECT facet, entry_kind, chunk_index, payload_ndjson, created_at
             FROM external_import_monitor_snapshot_chunks
             WHERE facet = {} AND entry_kind = {} AND ({} IS NULL OR chunk_index > {})
             ORDER BY chunk_index ASC
             LIMIT {}",
            &[
                SqlArg::Text(facet.as_str().to_string()),
                SqlArg::Text(entry_kind.as_str().to_string()),
                SqlArg::OptI32(after_chunk_index),
                SqlArg::OptI32(after_chunk_index),
                SqlArg::I32(limit),
            ],
        )
        .await
    }

    async fn delete_external_import_monitor_snapshot_chunks(
        &self,
        facet: MediaFacet,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_external_import_monitor_snapshot_chunks",
            "DELETE FROM external_import_monitor_snapshot_chunks WHERE facet = {}".to_string(),
            vec![SqlArg::Text(facet.as_str().to_string())],
        )
        .await
        .map_err(map_snapshot_chunk_error)?;
        Ok(())
    }
}
