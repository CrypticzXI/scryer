CREATE TABLE IF NOT EXISTS upstream_scheduler_states (
    host_key TEXT NOT NULL,
    destination_key TEXT NOT NULL,
    account_quota_key TEXT NOT NULL DEFAULT '',
    api_current BIGINT,
    api_max BIGINT,
    grab_current BIGINT,
    grab_max BIGINT,
    cooldown_until TIMESTAMPTZ,
    last_decision TEXT,
    last_feedback_at TIMESTAMPTZ,
    last_successful_at TIMESTAMPTZ,
    last_attempt_at TIMESTAMPTZ,
    admitted_count BIGINT NOT NULL DEFAULT 0,
    deferred_count BIGINT NOT NULL DEFAULT 0,
    skipped_count BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (host_key, destination_key, account_quota_key)
);

CREATE INDEX IF NOT EXISTS idx_upstream_scheduler_states_destination
    ON upstream_scheduler_states (destination_key, cooldown_until);

CREATE TABLE IF NOT EXISTS upstream_scheduler_rss_cadence (
    account_quota_key TEXT NOT NULL,
    destination_key TEXT NOT NULL,
    last_successful_poll_at TIMESTAMPTZ,
    last_attempt_at TIMESTAMPTZ,
    target_interval_seconds BIGINT NOT NULL,
    latest_safe_poll_at TIMESTAMPTZ,
    estimated_feed_depth BIGINT,
    freshness_risk DOUBLE PRECISION NOT NULL DEFAULT 0,
    destination_recent_activity_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (account_quota_key, destination_key)
);

CREATE INDEX IF NOT EXISTS idx_upstream_scheduler_rss_latest_safe_poll
    ON upstream_scheduler_rss_cadence (latest_safe_poll_at);
