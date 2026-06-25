ALTER TABLE discovery_sync_state
    ADD COLUMN transient_failure_count INTEGER NOT NULL DEFAULT 0;
