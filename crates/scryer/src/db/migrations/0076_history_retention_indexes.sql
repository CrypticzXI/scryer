CREATE INDEX IF NOT EXISTS idx_history_events_occurred_at
    ON history_events (occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_imports_status_updated_at
    ON imports (status, updated_at);

CREATE INDEX IF NOT EXISTS idx_rule_set_history_created_at
    ON rule_set_history (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_release_decisions_created_at
    ON release_decisions (created_at DESC);
