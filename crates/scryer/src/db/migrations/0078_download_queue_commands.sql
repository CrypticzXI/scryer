CREATE TABLE IF NOT EXISTS download_queue_commands (
    id TEXT PRIMARY KEY,
    action TEXT NOT NULL,
    client_type TEXT NOT NULL,
    download_client_item_id TEXT NOT NULL,
    is_history INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL,
    error_text TEXT,
    requested_by_user_id TEXT,
    started_at TEXT,
    finished_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_download_queue_commands_active_unique
ON download_queue_commands(action, client_type, download_client_item_id, is_history)
WHERE status IN ('queued', 'running');

CREATE INDEX IF NOT EXISTS idx_download_queue_commands_status
ON download_queue_commands(action, status, updated_at);

CREATE INDEX IF NOT EXISTS idx_download_queue_commands_source
ON download_queue_commands(client_type, download_client_item_id, is_history, created_at DESC);
