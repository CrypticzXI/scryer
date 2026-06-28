ALTER TABLE external_import_monitor_snapshot_chunks
    ADD COLUMN IF NOT EXISTS session_id text NOT NULL DEFAULT 'external-import-monitor-apply';

ALTER TABLE external_import_monitor_snapshot_chunks
    DROP CONSTRAINT IF EXISTS external_import_monitor_snapshot_chunks_pkey1;

ALTER TABLE external_import_monitor_snapshot_chunks
    DROP CONSTRAINT IF EXISTS external_import_monitor_snapshot_chunks_pkey;

ALTER TABLE external_import_monitor_snapshot_chunks
    ADD PRIMARY KEY (session_id, facet, entry_kind, chunk_index);
