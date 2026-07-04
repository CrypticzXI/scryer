CREATE TABLE IF NOT EXISTS upstream_scheduler_states (
    host_key TEXT NOT NULL,
    destination_key TEXT NOT NULL,
    account_quota_key TEXT NOT NULL DEFAULT '',
    api_current INTEGER,
    api_max INTEGER,
    grab_current INTEGER,
    grab_max INTEGER,
    quota_observed_at TEXT,
    quota_probe_after TEXT,
    quota_reset_at TEXT,
    quota_source TEXT,
    cooldown_until TEXT,
    last_decision TEXT,
    last_feedback_at TEXT,
    last_successful_at TEXT,
    last_attempt_at TEXT,
    admitted_count INTEGER NOT NULL DEFAULT 0,
    deferred_count INTEGER NOT NULL DEFAULT 0,
    skipped_count INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (host_key, destination_key, account_quota_key)
);

CREATE INDEX IF NOT EXISTS idx_upstream_scheduler_states_destination
    ON upstream_scheduler_states (destination_key, cooldown_until);

CREATE TABLE IF NOT EXISTS upstream_scheduler_rss_cadence (
    account_quota_key TEXT NOT NULL,
    destination_key TEXT NOT NULL,
    last_successful_poll_at TEXT,
    last_attempt_at TEXT,
    target_interval_seconds INTEGER NOT NULL,
    latest_safe_poll_at TEXT,
    estimated_feed_depth INTEGER,
    freshness_risk REAL NOT NULL DEFAULT 0,
    destination_recent_activity_at TEXT,
    last_seen_release_identity TEXT,
    last_seen_release_published_at TEXT,
    last_feed_gap_start_at TEXT,
    last_feed_gap_end_at TEXT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (account_quota_key, destination_key)
);

CREATE INDEX IF NOT EXISTS idx_upstream_scheduler_rss_latest_safe_poll
    ON upstream_scheduler_rss_cadence (latest_safe_poll_at);
