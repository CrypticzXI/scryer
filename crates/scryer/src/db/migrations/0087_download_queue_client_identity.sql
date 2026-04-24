-- Queue/submission identities must include the configured download client, not
-- just the native client type. Native clients can reuse item IDs across
-- multiple configured instances.

DROP INDEX IF EXISTS idx_download_submissions_title_request_signature;

CREATE TABLE download_submissions_new (
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    download_client_id TEXT NOT NULL DEFAULT '',
    download_client_type TEXT NOT NULL,
    download_client_item_id TEXT NOT NULL,
    source_title TEXT,
    submitted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    collection_id TEXT,
    tracked_state TEXT,
    tracked_state_at TEXT,
    source_hint TEXT,
    source_kind TEXT,
    request_signature TEXT,
    episode_id TEXT,
    UNIQUE(download_client_id, download_client_type, download_client_item_id)
);

INSERT INTO download_submissions_new (
    id,
    title_id,
    facet,
    download_client_id,
    download_client_type,
    download_client_item_id,
    source_title,
    submitted_at,
    collection_id,
    tracked_state,
    tracked_state_at,
    source_hint,
    source_kind,
    request_signature,
    episode_id
)
SELECT
    id,
    title_id,
    facet,
    '',
    download_client_type,
    download_client_item_id,
    source_title,
    submitted_at,
    collection_id,
    tracked_state,
    tracked_state_at,
    source_hint,
    source_kind,
    request_signature,
    episode_id
FROM download_submissions;

DROP TABLE download_submissions;
ALTER TABLE download_submissions_new RENAME TO download_submissions;

CREATE INDEX IF NOT EXISTS idx_download_submissions_title_request_signature
    ON download_submissions(title_id, request_signature);

DROP INDEX IF EXISTS idx_download_queue_commands_active_unique;
DROP INDEX IF EXISTS idx_download_queue_commands_source;

ALTER TABLE download_queue_commands ADD COLUMN client_id TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_download_queue_commands_active_unique
ON download_queue_commands(action, COALESCE(client_id, ''), client_type, download_client_item_id, is_history)
WHERE status IN ('queued', 'running');

CREATE INDEX IF NOT EXISTS idx_download_queue_commands_source
ON download_queue_commands(COALESCE(client_id, ''), client_type, download_client_item_id, is_history, created_at DESC);
