ALTER TABLE workflow_operations RENAME TO workflow_operations__old_0122;

CREATE TABLE workflow_operations(
    id TEXT PRIMARY KEY,
    operation_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    actor_user_id TEXT,
    title_id TEXT,
    collection_id TEXT,
    episode_id TEXT,
    release_id TEXT,
    media_file_id TEXT,
    external_reference TEXT,
    progress_json TEXT,
    started_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    job_key TEXT,
    trigger_source TEXT,
    summary_json TEXT,
    summary_text TEXT,
    error_text TEXT,
    FOREIGN KEY (actor_user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL,
    FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE SET NULL,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL,
    FOREIGN KEY (media_file_id) REFERENCES media_files(id) ON DELETE SET NULL
);

INSERT INTO workflow_operations (
    id,
    operation_type,
    status,
    actor_user_id,
    title_id,
    collection_id,
    episode_id,
    release_id,
    media_file_id,
    external_reference,
    progress_json,
    started_at,
    completed_at,
    created_at,
    updated_at,
    job_key,
    trigger_source,
    summary_json,
    summary_text,
    error_text
)
SELECT
    id,
    operation_type,
    status,
    actor_user_id,
    title_id,
    collection_id,
    episode_id,
    release_id,
    media_file_id,
    external_reference,
    progress_json,
    started_at,
    completed_at,
    created_at,
    updated_at,
    job_key,
    trigger_source,
    summary_json,
    summary_text,
    error_text
FROM workflow_operations__old_0122;

DROP TABLE workflow_operations__old_0122;

CREATE INDEX idx_operations_status_time
    ON workflow_operations (status, started_at DESC);
CREATE INDEX idx_workflow_operations_job_key_started
    ON workflow_operations (job_key, started_at DESC);
CREATE INDEX idx_workflow_operations_job_key_status
    ON workflow_operations (job_key, status, started_at DESC);
CREATE INDEX idx_workflow_operations_status_started
    ON workflow_operations (status, started_at);

DROP TABLE IF EXISTS download_jobs;
DROP TABLE IF EXISTS integration_tokens;
DROP TABLE IF EXISTS push_subscriptions;
DROP TABLE IF EXISTS quarantine_items;
DROP TABLE IF EXISTS releases;
DROP TABLE IF EXISTS scheduler_jobs;
DROP TABLE IF EXISTS title_aliases;
DROP TABLE IF EXISTS upgrades;
