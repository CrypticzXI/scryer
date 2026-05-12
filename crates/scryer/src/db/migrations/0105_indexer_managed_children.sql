ALTER TABLE indexers ADD COLUMN managed_parent_config_id TEXT;
ALTER TABLE indexers ADD COLUMN managed_child_key TEXT;
ALTER TABLE indexers ADD COLUMN managed_metadata_json TEXT;

CREATE INDEX IF NOT EXISTS idx_indexers_managed_parent ON indexers(managed_parent_config_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_indexers_managed_child_identity
ON indexers(managed_parent_config_id, managed_child_key)
WHERE managed_parent_config_id IS NOT NULL AND managed_child_key IS NOT NULL;