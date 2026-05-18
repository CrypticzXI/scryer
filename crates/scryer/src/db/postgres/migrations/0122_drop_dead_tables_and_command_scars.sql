ALTER TABLE workflow_operations
    DROP CONSTRAINT IF EXISTS workflow_operations_release_id_fkey;

DROP TABLE IF EXISTS download_jobs;
DROP TABLE IF EXISTS integration_tokens;
DROP TABLE IF EXISTS push_subscriptions;
DROP TABLE IF EXISTS quarantine_items;
DROP TABLE IF EXISTS releases;
DROP TABLE IF EXISTS scheduler_jobs;
DROP TABLE IF EXISTS title_aliases;
DROP TABLE IF EXISTS upgrades;
