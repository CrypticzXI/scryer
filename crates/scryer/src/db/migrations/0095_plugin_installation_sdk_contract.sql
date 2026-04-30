ALTER TABLE plugin_installations ADD COLUMN sdk_version TEXT NOT NULL DEFAULT '';
ALTER TABLE plugin_installations ADD COLUMN sdk_constraint TEXT NOT NULL DEFAULT '';
ALTER TABLE plugin_installations ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'downloaded';

UPDATE plugin_installations
SET source_kind = 'bundled'
WHERE is_builtin = 1
  AND wasm_bytes IS NULL;
