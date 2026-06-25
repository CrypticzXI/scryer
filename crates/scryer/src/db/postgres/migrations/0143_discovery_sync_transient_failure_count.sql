ALTER TABLE discovery_sync_state
    ADD COLUMN transient_failure_count BIGINT NOT NULL DEFAULT 0;
