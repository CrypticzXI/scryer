ALTER TABLE discovery_sync_state
    ADD COLUMN inflight_context_snapshot_run_id TEXT;

ALTER TABLE discovery_sync_state
    ADD COLUMN lease_owner_id TEXT;

ALTER TABLE discovery_sync_state
    ADD COLUMN lease_expires_at TIMESTAMPTZ;
