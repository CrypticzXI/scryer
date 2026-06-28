CREATE INDEX IF NOT EXISTS idx_workflow_operations_job_recent_started
    ON workflow_operations (started_at DESC)
    WHERE job_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_workflow_operations_actor_recent_started
    ON workflow_operations (actor_user_id, started_at DESC)
    WHERE job_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_workflow_operations_actor_job_started
    ON workflow_operations (actor_user_id, job_key, started_at DESC)
    WHERE job_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_workflow_operations_active_job_started
    ON workflow_operations (started_at ASC)
    WHERE job_key IS NOT NULL
      AND status IN ('queued', 'running', 'discovering');
