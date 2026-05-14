-- Hard cutover PostgreSQL to the same logical schema used by SQLite. PostgreSQL
-- may keep native JSONB/BOOLEAN/TIMESTAMPTZ/BYTEA types, but the table and
-- column contract should otherwise match SQLite.

DELETE FROM plugin_installations
 WHERE plugin_id = '__registry_cache'
    OR plugin_type = '__cache';

-- Titles: move the metadata that lived only in record_json into first-class
-- columns, then remove the pre-launch snapshot columns.
ALTER TABLE titles
    ADD COLUMN IF NOT EXISTS name_normalized TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'active',
    ADD COLUMN IF NOT EXISTS created_by TEXT,
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS year INTEGER,
    ADD COLUMN IF NOT EXISTS overview TEXT,
    ADD COLUMN IF NOT EXISTS poster_url TEXT,
    ADD COLUMN IF NOT EXISTS sort_title TEXT,
    ADD COLUMN IF NOT EXISTS imdb_id TEXT,
    ADD COLUMN IF NOT EXISTS runtime_minutes INTEGER,
    ADD COLUMN IF NOT EXISTS genres JSONB NOT NULL DEFAULT '[]'::JSONB,
    ADD COLUMN IF NOT EXISTS content_status TEXT,
    ADD COLUMN IF NOT EXISTS language TEXT,
    ADD COLUMN IF NOT EXISTS first_aired TEXT,
    ADD COLUMN IF NOT EXISTS network TEXT,
    ADD COLUMN IF NOT EXISTS studio TEXT,
    ADD COLUMN IF NOT EXISTS country TEXT,
    ADD COLUMN IF NOT EXISTS aliases JSONB NOT NULL DEFAULT '[]'::JSONB,
    ADD COLUMN IF NOT EXISTS metadata_language TEXT,
    ADD COLUMN IF NOT EXISTS metadata_fetched_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS min_availability TEXT,
    ADD COLUMN IF NOT EXISTS digital_release_date TEXT,
    ADD COLUMN IF NOT EXISTS banner_url TEXT,
    ADD COLUMN IF NOT EXISTS background_url TEXT,
    ADD COLUMN IF NOT EXISTS tagged_aliases_json JSONB DEFAULT '[]'::JSONB;

UPDATE titles
   SET library_id = COALESCE(NULLIF(record_json->>'library_id', ''), library_id, ''),
       name = COALESCE(NULLIF(record_json->>'name', ''), name),
       facet = COALESCE(NULLIF(record_json->>'facet', ''), facet),
       monitored = COALESCE((record_json->>'monitored')::BOOLEAN, monitored, TRUE),
       tags = COALESCE(record_json->'tags', tags, '[]'::JSONB),
       external_ids = COALESCE(record_json->'external_ids', external_ids, '[]'::JSONB),
       created_by = COALESCE(record_json->>'created_by', created_by),
       created_at = COALESCE(NULLIF(record_json->>'created_at', '')::TIMESTAMPTZ, created_at, NOW()),
       year = COALESCE(NULLIF(record_json->>'year', '')::INTEGER, year),
       overview = COALESCE(record_json->>'overview', overview),
       poster_url = COALESCE(record_json->>'poster_url', poster_url),
       banner_url = COALESCE(record_json->>'banner_url', banner_url),
       background_url = COALESCE(record_json->>'background_url', background_url),
       sort_title = COALESCE(record_json->>'sort_title', sort_title),
       slug = COALESCE(record_json->>'slug', slug),
       imdb_id = COALESCE(record_json->>'imdb_id', imdb_id),
       runtime_minutes = COALESCE(NULLIF(record_json->>'runtime_minutes', '')::INTEGER, runtime_minutes),
       genres = COALESCE(record_json->'genres', genres, '[]'::JSONB),
       content_status = COALESCE(record_json->>'content_status', content_status),
       language = COALESCE(record_json->>'language', language),
       first_aired = COALESCE(record_json->>'first_aired', first_aired),
       network = COALESCE(record_json->>'network', network),
       studio = COALESCE(record_json->>'studio', studio),
       country = COALESCE(record_json->>'country', country),
       aliases = COALESCE(record_json->'aliases', aliases, '[]'::JSONB),
       tagged_aliases_json = COALESCE(record_json->'tagged_aliases', tagged_aliases_json, '[]'::JSONB),
       metadata_language = COALESCE(record_json->>'metadata_language', metadata_language),
       metadata_fetched_at = COALESCE(NULLIF(record_json->>'metadata_fetched_at', '')::TIMESTAMPTZ, metadata_fetched_at),
       min_availability = COALESCE(record_json->>'min_availability', min_availability),
       digital_release_date = COALESCE(record_json->>'digital_release_date', digital_release_date),
       folder_path = COALESCE(record_json->>'folder_path', folder_path),
       updated_at = COALESCE(updated_at, NOW())
 WHERE record_json IS NOT NULL;

ALTER TABLE titles
    ALTER COLUMN library_id SET NOT NULL,
    ALTER COLUMN tags SET NOT NULL,
    ALTER COLUMN external_ids SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN genres SET NOT NULL,
    ALTER COLUMN aliases SET NOT NULL,
    ALTER COLUMN metadata_hydration_attempt_count SET NOT NULL,
    DROP COLUMN IF EXISTS metadata_json,
    DROP COLUMN IF EXISTS record_json;

-- Config tables: match SQLite's column contract and encrypted string storage.
ALTER TABLE indexers RENAME COLUMN config_json TO config_json_legacy_jsonb;
ALTER TABLE indexers
    ADD COLUMN IF NOT EXISTS api_key TEXT,
    ADD COLUMN IF NOT EXISTS status TEXT,
    ADD COLUMN IF NOT EXISTS last_seen_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS record_json JSONB NOT NULL DEFAULT '{}'::JSONB,
    ADD COLUMN IF NOT EXISTS api_key_encrypted TEXT,
    ADD COLUMN IF NOT EXISTS rate_limit_seconds BIGINT,
    ADD COLUMN IF NOT EXISTS rate_limit_burst BIGINT,
    ADD COLUMN IF NOT EXISTS disabled_until TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS enable_interactive_search BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN IF NOT EXISTS enable_auto_search BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN IF NOT EXISTS managed_parent_config_id TEXT,
    ADD COLUMN IF NOT EXISTS managed_child_key TEXT,
    ADD COLUMN IF NOT EXISTS managed_metadata_json TEXT,
    ADD COLUMN IF NOT EXISTS last_health_status TEXT,
    ADD COLUMN IF NOT EXISTS last_error_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS config_json TEXT;

ALTER TABLE indexers
    ALTER COLUMN managed_metadata_json TYPE TEXT USING managed_metadata_json::TEXT;

UPDATE indexers
   SET base_url = COALESCE(NULLIF(record_json->>'base_url', ''), base_url, ''),
       api_key_encrypted = COALESCE(record_json->>'api_key_encrypted', api_key_encrypted, api_key),
       rate_limit_seconds = COALESCE(NULLIF(record_json->>'rate_limit_seconds', '')::BIGINT, rate_limit_seconds),
       rate_limit_burst = COALESCE(NULLIF(record_json->>'rate_limit_burst', '')::BIGINT, rate_limit_burst),
       disabled_until = COALESCE(NULLIF(record_json->>'disabled_until', '')::TIMESTAMPTZ, disabled_until),
       enable_interactive_search = COALESCE((record_json->>'enable_interactive_search')::BOOLEAN, enable_interactive_search, TRUE),
       enable_auto_search = COALESCE((record_json->>'enable_auto_search')::BOOLEAN, enable_auto_search, TRUE),
       managed_parent_config_id = COALESCE(record_json->>'managed_parent_config_id', managed_parent_config_id),
       managed_child_key = COALESCE(record_json->>'managed_child_key', managed_child_key),
       managed_metadata_json = COALESCE(record_json->>'managed_metadata_json', managed_metadata_json),
       last_health_status = COALESCE(record_json->>'last_health_status', last_health_status, status),
       last_error_at = COALESCE(NULLIF(record_json->>'last_error_at', '')::TIMESTAMPTZ, last_error_at, last_seen_at),
       config_json = COALESCE(record_json->>'config_json', config_json, config_json_legacy_jsonb::TEXT),
       created_at = COALESCE(created_at, NOW()),
       updated_at = COALESCE(updated_at, NOW());

ALTER TABLE indexers
    ALTER COLUMN base_url SET NOT NULL,
    ALTER COLUMN is_enabled SET NOT NULL,
    ALTER COLUMN enable_interactive_search SET NOT NULL,
    ALTER COLUMN enable_auto_search SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET NOT NULL,
    DROP COLUMN IF EXISTS api_key,
    DROP COLUMN IF EXISTS status,
    DROP COLUMN IF EXISTS last_error,
    DROP COLUMN IF EXISTS last_seen_at,
    DROP COLUMN IF EXISTS record_json,
    DROP COLUMN IF EXISTS config_json_legacy_jsonb;

ALTER TABLE download_clients RENAME COLUMN config_json TO config_json_legacy_jsonb;
ALTER TABLE download_clients
    ADD COLUMN IF NOT EXISTS record_json JSONB NOT NULL DEFAULT '{}'::JSONB,
    ADD COLUMN IF NOT EXISTS config_json TEXT;

UPDATE download_clients
   SET config_json = COALESCE(record_json->>'config_json', config_json, config_json_legacy_jsonb::TEXT, '{}'),
       created_at = COALESCE(created_at, NOW()),
       updated_at = COALESCE(updated_at, NOW());

ALTER TABLE download_clients
    ALTER COLUMN config_json SET NOT NULL,
    ALTER COLUMN is_enabled SET NOT NULL,
    ALTER COLUMN status SET NOT NULL,
    ALTER COLUMN client_priority SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET NOT NULL,
    DROP COLUMN IF EXISTS record_json,
    DROP COLUMN IF EXISTS config_json_legacy_jsonb;

ALTER TABLE subtitle_provider_configs RENAME COLUMN config_json TO config_json_legacy_jsonb;
ALTER TABLE subtitle_provider_configs
    ALTER COLUMN config_json_legacy_jsonb TYPE TEXT USING config_json_legacy_jsonb::TEXT;
ALTER TABLE subtitle_provider_configs
    ADD COLUMN IF NOT EXISTS record_json JSONB NOT NULL DEFAULT '{}'::JSONB,
    ADD COLUMN IF NOT EXISTS status TEXT,
    ADD COLUMN IF NOT EXISTS last_seen_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS config_json TEXT,
    ADD COLUMN IF NOT EXISTS enabled_facets JSONB NOT NULL DEFAULT '[]'::JSONB;

DO $$
BEGIN
    IF to_regclass('subtitle_providers') IS NOT NULL THEN
        INSERT INTO subtitle_provider_configs (
            id, name, provider_type, config_json_legacy_jsonb, record_json,
            is_enabled, last_health_status, last_error, last_error_at,
            created_at, updated_at, enabled_facets
        )
        SELECT id, name, provider_type, config_json::TEXT, record_json,
               is_enabled, status, last_error, last_seen_at,
               created_at, updated_at, '[]'::JSONB
          FROM subtitle_providers
        ON CONFLICT (id) DO NOTHING;
    END IF;
END $$;

DROP TABLE IF EXISTS subtitle_providers;

UPDATE subtitle_provider_configs
   SET name = COALESCE(record_json->>'name', name, ''),
       provider_type = COALESCE(record_json->>'provider_type', provider_type, ''),
       config_json = COALESCE(record_json->>'config_json', config_json, config_json_legacy_jsonb::TEXT, '{}'),
       enabled_facets = COALESCE(record_json->'enabled_facets', enabled_facets, '[]'::JSONB),
       last_health_status = COALESCE(record_json->>'last_health_status', last_health_status, status),
       last_error_at = COALESCE(NULLIF(record_json->>'last_error_at', '')::TIMESTAMPTZ, last_error_at, last_seen_at),
       disabled_until = COALESCE(NULLIF(record_json->>'disabled_until', '')::TIMESTAMPTZ, disabled_until),
       created_at = COALESCE(created_at, NOW()),
       updated_at = COALESCE(updated_at, NOW());

ALTER TABLE subtitle_provider_configs
    ALTER COLUMN name SET NOT NULL,
    ALTER COLUMN provider_type SET NOT NULL,
    ALTER COLUMN config_json SET NOT NULL,
    ALTER COLUMN is_enabled SET NOT NULL,
    ALTER COLUMN enabled_facets SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET NOT NULL,
    DROP COLUMN IF EXISTS status,
    DROP COLUMN IF EXISTS last_seen_at,
    DROP COLUMN IF EXISTS record_json,
    DROP COLUMN IF EXISTS config_json_legacy_jsonb;

-- Customization tables.
ALTER TABLE rule_sets
    ADD COLUMN IF NOT EXISTS description TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS rego_source TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS applied_facets JSONB NOT NULL DEFAULT '[]'::JSONB,
    ADD COLUMN IF NOT EXISTS managed_key TEXT,
    ADD COLUMN IF NOT EXISTS is_managed BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE rule_sets
   SET name = COALESCE(record_json->>'name', name, ''),
       description = COALESCE(record_json->>'description', description, ''),
       rego_source = COALESCE(record_json->>'rego_source', rego_source, rule_json::TEXT, ''),
       enabled = COALESCE((record_json->>'enabled')::BOOLEAN, enabled, TRUE),
       priority = COALESCE(NULLIF(record_json->>'priority', '')::INTEGER, priority, 0),
       applied_facets = COALESCE(record_json->'applied_facets', applied_facets, '[]'::JSONB),
       created_at = COALESCE(NULLIF(record_json->>'created_at', '')::TIMESTAMPTZ, created_at, NOW()),
       updated_at = COALESCE(NULLIF(record_json->>'updated_at', '')::TIMESTAMPTZ, updated_at, NOW()),
       is_managed = COALESCE((record_json->>'is_managed')::BOOLEAN, is_managed, FALSE),
       managed_key = COALESCE(record_json->>'managed_key', managed_key);

ALTER TABLE rule_sets
    ALTER COLUMN name SET NOT NULL,
    ALTER COLUMN description SET NOT NULL,
    ALTER COLUMN rego_source SET NOT NULL,
    ALTER COLUMN enabled SET NOT NULL,
    ALTER COLUMN priority TYPE INTEGER USING priority::INTEGER,
    ALTER COLUMN priority SET NOT NULL,
    ALTER COLUMN applied_facets SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET NOT NULL,
    ALTER COLUMN is_managed SET NOT NULL,
    DROP COLUMN IF EXISTS rule_json,
    DROP COLUMN IF EXISTS record_json;

ALTER TABLE post_processing_scripts
    ADD COLUMN IF NOT EXISTS description TEXT DEFAULT '',
    ADD COLUMN IF NOT EXISTS script_type TEXT NOT NULL DEFAULT 'inline',
    ADD COLUMN IF NOT EXISTS script_content TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS applied_facets JSONB NOT NULL DEFAULT '[]'::JSONB,
    ADD COLUMN IF NOT EXISTS execution_mode TEXT NOT NULL DEFAULT 'blocking',
    ADD COLUMN IF NOT EXISTS timeout_secs BIGINT DEFAULT 300,
    ADD COLUMN IF NOT EXISTS enabled BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN IF NOT EXISTS debug BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE post_processing_scripts
   SET name = COALESCE(record_json->>'name', name, ''),
       description = COALESCE(record_json->>'description', description, ''),
       script_type = COALESCE(record_json->>'script_type', script_type, 'inline'),
       script_content = COALESCE(record_json->>'script_content', script_content, script_path, ''),
       applied_facets = COALESCE(record_json->'applied_facets', applied_facets, '[]'::JSONB),
       execution_mode = COALESCE(record_json->>'execution_mode', execution_mode, 'blocking'),
       timeout_secs = COALESCE(NULLIF(record_json->>'timeout_secs', '')::BIGINT, timeout_secs, 300),
       enabled = COALESCE((record_json->>'enabled')::BOOLEAN, enabled, is_enabled, TRUE),
       debug = COALESCE((record_json->>'debug')::BOOLEAN, debug, FALSE),
       created_at = COALESCE(NULLIF(record_json->>'created_at', '')::TIMESTAMPTZ, created_at, NOW()),
       updated_at = COALESCE(NULLIF(record_json->>'updated_at', '')::TIMESTAMPTZ, updated_at, NOW());

ALTER TABLE post_processing_scripts
    ALTER COLUMN name SET NOT NULL,
    ALTER COLUMN script_type SET NOT NULL,
    ALTER COLUMN script_content SET NOT NULL,
    ALTER COLUMN applied_facets SET NOT NULL,
    ALTER COLUMN execution_mode SET NOT NULL,
    ALTER COLUMN priority TYPE INTEGER USING priority::INTEGER,
    ALTER COLUMN priority SET NOT NULL,
    ALTER COLUMN enabled SET NOT NULL,
    ALTER COLUMN debug SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET NOT NULL,
    DROP COLUMN IF EXISTS script_path,
    DROP COLUMN IF EXISTS is_enabled,
    DROP COLUMN IF EXISTS record_json;

ALTER TABLE post_processing_script_runs
    ADD COLUMN IF NOT EXISTS script_name TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS title_id TEXT,
    ADD COLUMN IF NOT EXISTS title_name TEXT,
    ADD COLUMN IF NOT EXISTS facet TEXT,
    ADD COLUMN IF NOT EXISTS file_path TEXT,
    ADD COLUMN IF NOT EXISTS exit_code INTEGER,
    ADD COLUMN IF NOT EXISTS stdout_tail TEXT,
    ADD COLUMN IF NOT EXISTS stderr_tail TEXT,
    ADD COLUMN IF NOT EXISTS duration_ms BIGINT,
    ADD COLUMN IF NOT EXISTS env_payload_json TEXT,
    ADD COLUMN IF NOT EXISTS completed_at TIMESTAMPTZ;

UPDATE post_processing_script_runs
   SET script_name = COALESCE(record_json->>'script_name', script_name, ''),
       title_id = COALESCE(record_json->>'title_id', title_id),
       title_name = COALESCE(record_json->>'title_name', title_name),
       facet = COALESCE(record_json->>'facet', facet),
       file_path = COALESCE(record_json->>'file_path', file_path),
       exit_code = COALESCE(NULLIF(record_json->>'exit_code', '')::INTEGER, exit_code),
       stdout_tail = COALESCE(record_json->>'stdout_tail', stdout_tail),
       stderr_tail = COALESCE(record_json->>'stderr_tail', stderr_tail, output_text),
       duration_ms = COALESCE(NULLIF(record_json->>'duration_ms', '')::BIGINT, duration_ms),
       env_payload_json = COALESCE(record_json->>'env_payload_json', env_payload_json),
       started_at = COALESCE(NULLIF(record_json->>'started_at', '')::TIMESTAMPTZ, started_at, created_at, NOW()),
       completed_at = COALESCE(NULLIF(record_json->>'completed_at', '')::TIMESTAMPTZ, completed_at, finished_at);

ALTER TABLE post_processing_script_runs
    ALTER COLUMN script_id SET NOT NULL,
    ALTER COLUMN script_name SET NOT NULL,
    ALTER COLUMN status SET NOT NULL,
    ALTER COLUMN started_at SET NOT NULL,
    DROP COLUMN IF EXISTS output_text,
    DROP COLUMN IF EXISTS finished_at,
    DROP COLUMN IF EXISTS created_at,
    DROP COLUMN IF EXISTS record_json;

-- Plugin tables: catalog state owns cache/status, not plugin_installations.
ALTER TABLE plugin_installations DROP COLUMN IF EXISTS record_json;

ALTER TABLE plugin_catalog_sources
    ALTER COLUMN catalog_json TYPE TEXT USING catalog_json::TEXT;
ALTER TABLE plugin_catalog_sources DROP COLUMN IF EXISTS record_json;

ALTER TABLE plugin_catalog_status
    ADD COLUMN IF NOT EXISTS catalog_json JSONB,
    ADD COLUMN IF NOT EXISTS record_json JSONB NOT NULL DEFAULT '{}'::JSONB,
    ADD COLUMN IF NOT EXISTS last_success_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS last_error TEXT,
    ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS status_json TEXT,
    ADD COLUMN IF NOT EXISTS checked_at TIMESTAMPTZ;

UPDATE plugin_catalog_status
   SET status_json = COALESCE(record_json->>'status_json', catalog_json::TEXT, status_json, '{}'),
       checked_at = COALESCE(NULLIF(record_json->>'checked_at', '')::TIMESTAMPTZ, checked_at, last_success_at, updated_at, NOW());

ALTER TABLE plugin_catalog_status
    ALTER COLUMN status_json SET NOT NULL,
    ALTER COLUMN checked_at SET NOT NULL,
    DROP COLUMN IF EXISTS catalog_json,
    DROP COLUMN IF EXISTS record_json,
    DROP COLUMN IF EXISTS last_success_at,
    DROP COLUMN IF EXISTS last_error,
    DROP COLUMN IF EXISTS updated_at;
