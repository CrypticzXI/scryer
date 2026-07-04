CREATE TABLE IF NOT EXISTS upstream_scheduler_states (
    host_key TEXT NOT NULL,
    destination_key TEXT NOT NULL,
    account_quota_key TEXT NOT NULL DEFAULT '',
    api_current BIGINT,
    api_max BIGINT,
    grab_current BIGINT,
    grab_max BIGINT,
    quota_observed_at TIMESTAMPTZ,
    quota_probe_after TIMESTAMPTZ,
    quota_reset_at TIMESTAMPTZ,
    quota_source TEXT,
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
    ON upstream_scheduler_states (destination_key);

CREATE TABLE IF NOT EXISTS upstream_destination_cooldowns (
    destination_key TEXT PRIMARY KEY,
    cooldown_until TIMESTAMPTZ NOT NULL,
    retry_after_seconds BIGINT,
    source TEXT NOT NULL,
    status_code BIGINT,
    message TEXT,
    observed_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_upstream_destination_cooldowns_until
    ON upstream_destination_cooldowns (cooldown_until);

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
    last_seen_release_identity TEXT,
    last_seen_release_published_at TIMESTAMPTZ,
    last_feed_gap_start_at TIMESTAMPTZ,
    last_feed_gap_end_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (account_quota_key, destination_key)
);

CREATE INDEX IF NOT EXISTS idx_upstream_scheduler_rss_latest_safe_poll
    ON upstream_scheduler_rss_cadence (latest_safe_poll_at);
