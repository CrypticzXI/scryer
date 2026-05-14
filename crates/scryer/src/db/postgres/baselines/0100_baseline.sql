CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    success BOOLEAN NOT NULL,
    checksum BYTEA NOT NULL,
    execution_time BIGINT NOT NULL,
    checksum_algo TEXT NOT NULL DEFAULT 'sha384',
    runtime_version TEXT NOT NULL DEFAULT '',
    error_message TEXT
);

CREATE TABLE IF NOT EXISTS settings_definitions (
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL,
    scope TEXT NOT NULL,
    key_name TEXT NOT NULL,
    data_type TEXT NOT NULL,
    default_value_json JSONB NOT NULL,
    is_sensitive BOOLEAN NOT NULL DEFAULT FALSE,
    validation_json JSONB,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (scope, key_name)
);

CREATE TABLE IF NOT EXISTS settings_values (
    id TEXT PRIMARY KEY,
    setting_definition_id TEXT NOT NULL REFERENCES settings_definitions(id) ON DELETE CASCADE,
    scope TEXT NOT NULL,
    scope_id TEXT,
    value_json JSONB,
    source TEXT,
    updated_by_user_id TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_settings_values_definition_scope
    ON settings_values(setting_definition_id, COALESCE(scope_id, ''));

CREATE TABLE IF NOT EXISTS quality_profiles_json (
    scope TEXT NOT NULL,
    scope_id TEXT,
    profiles_json JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_quality_profiles_json_scope
    ON quality_profiles_json(scope, COALESCE(scope_id, ''));

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    entitlements JSONB NOT NULL DEFAULT '[]'::JSONB,
    password_hash TEXT
);

CREATE TABLE IF NOT EXISTS titles (
    id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL DEFAULT '',
    name TEXT NOT NULL,
    monitored BOOLEAN NOT NULL DEFAULT TRUE,
    facet TEXT NOT NULL,
    tags JSONB NOT NULL DEFAULT '[]'::JSONB,
    external_ids JSONB NOT NULL DEFAULT '[]'::JSONB,
    created_by TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    year INTEGER,
    overview TEXT,
    poster_url TEXT,
    poster_local_path TEXT,
    banner_url TEXT,
    banner_local_path TEXT,
    background_url TEXT,
    background_local_path TEXT,
    sort_title TEXT,
    slug TEXT,
    imdb_id TEXT,
    runtime_minutes INTEGER,
    genres JSONB NOT NULL DEFAULT '[]'::JSONB,
    content_status TEXT,
    language TEXT,
    first_aired TEXT,
    network TEXT,
    studio TEXT,
    country TEXT,
    aliases JSONB NOT NULL DEFAULT '[]'::JSONB,
    metadata_language TEXT,
    metadata_fetched_at TIMESTAMPTZ,
    min_availability TEXT,
    digital_release_date TEXT,
    folder_path TEXT,
    tagged_aliases_json JSONB NOT NULL DEFAULT '[]'::JSONB,
    metadata_hydration_next_attempt_at TIMESTAMPTZ,
    metadata_hydration_attempt_count BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_titles_metadata_hydration_due
    ON titles(metadata_hydration_next_attempt_at, id)
    WHERE metadata_hydration_next_attempt_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS title_external_ids (
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    facet TEXT NOT NULL,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    provenance TEXT NOT NULL,
    source_scope TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS collections (
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    collection_type TEXT NOT NULL,
    collection_index TEXT NOT NULL,
    label TEXT,
    ordered_path TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS episodes (
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    collection_id TEXT REFERENCES collections(id) ON DELETE SET NULL,
    episode_type TEXT NOT NULL,
    episode_number TEXT,
    season_number TEXT,
    episode_label TEXT,
    title TEXT,
    air_date TEXT,
    duration_seconds BIGINT,
    monitored BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ
);

CREATE TABLE IF NOT EXISTS title_images (
    id TEXT PRIMARY KEY,
    title_id TEXT,
    provider TEXT,
    provider_image_id TEXT,
    kind TEXT,
    source_url TEXT,
    source_etag TEXT,
    source_last_modified TEXT,
    source_format TEXT,
    source_width BIGINT,
    source_height BIGINT,
    storage_mode TEXT,
    master_path TEXT,
    master_format TEXT,
    master_sha256 TEXT,
    master_width BIGINT,
    master_height BIGINT,
    bytes BYTEA,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE (title_id, kind)
);

CREATE TABLE IF NOT EXISTS title_image_variants (
    id TEXT PRIMARY KEY,
    title_image_id TEXT,
    variant_key TEXT,
    path TEXT,
    format TEXT,
    width BIGINT,
    height BIGINT,
    bytes BYTEA,
    sha256 TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE (title_image_id, variant_key)
);

CREATE TABLE IF NOT EXISTS library_scan_unmatched_items (
    id TEXT PRIMARY KEY,
    facet TEXT NOT NULL,
    title_id TEXT,
    scan_root TEXT,
    item_path TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    metadata_json JSONB,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS indexers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    base_url TEXT NOT NULL,
    api_key_encrypted TEXT,
    rate_limit_seconds BIGINT,
    rate_limit_burst BIGINT,
    disabled_until TIMESTAMPTZ,
    is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    enable_interactive_search BOOLEAN NOT NULL DEFAULT TRUE,
    enable_auto_search BOOLEAN NOT NULL DEFAULT TRUE,
    managed_parent_config_id TEXT,
    managed_child_key TEXT,
    managed_metadata_json TEXT,
    last_health_status TEXT,
    last_error_at TIMESTAMPTZ,
    config_json TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS download_clients (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    client_type TEXT NOT NULL,
    base_url TEXT,
    config_json TEXT,
    is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    status TEXT NOT NULL DEFAULT 'idle',
    last_error TEXT,
    last_seen_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    client_priority BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS subtitle_provider_configs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    config_json TEXT NOT NULL DEFAULT '{}',
    enabled_facets JSONB NOT NULL DEFAULT '[]'::JSONB,
    is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    last_error TEXT,
    last_health_status TEXT,
    last_error_at TIMESTAMPTZ,
    disabled_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS subtitle_providers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    config_json JSONB,
    record_json JSONB NOT NULL DEFAULT '{}'::JSONB,
    is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    status TEXT NOT NULL DEFAULT 'idle',
    last_error TEXT,
    last_seen_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS plugin_installations (
    id TEXT PRIMARY KEY,
    plugin_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    version TEXT NOT NULL,
    sdk_version TEXT NOT NULL,
    sdk_constraint TEXT NOT NULL,
    scryer_constraint TEXT,
    plugin_type TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    is_builtin BOOLEAN NOT NULL DEFAULT FALSE,
    wasm_bytes BYTEA,
    wasm_encoding TEXT NOT NULL DEFAULT 'identity',
    wasm_digest_algo TEXT,
    source_url TEXT,
    support_tier TEXT NOT NULL DEFAULT 'official',
    publisher TEXT,
    docs_url TEXT,
    source_repo TEXT,
    manifest_url TEXT,
    wasm_digest TEXT,
    artifact_digest TEXT,
    descriptor_json JSONB,
    installed_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS plugin_catalog_sources (
    source_key TEXT PRIMARY KEY,
    source_kind TEXT NOT NULL,
    source_url TEXT NOT NULL,
    github_repo TEXT,
    support_tier TEXT NOT NULL,
    catalog_json TEXT,
    last_success_at TIMESTAMPTZ,
    last_error TEXT,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS plugin_catalog_status (
    status_key TEXT PRIMARY KEY,
    status_json TEXT NOT NULL,
    checked_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS blocklist (
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL,
    source_title TEXT,
    source_hint TEXT,
    quality TEXT,
    download_id TEXT,
    reason TEXT,
    data_json JSONB,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS release_download_attempts (
    id TEXT PRIMARY KEY,
    title_id TEXT,
    source_hint TEXT,
    source_title TEXT,
    outcome TEXT NOT NULL,
    error_message TEXT,
    source_password TEXT,
    attempted_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS download_submissions (
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL DEFAULT '',
    facet TEXT NOT NULL DEFAULT '',
    download_client_id TEXT NOT NULL DEFAULT '',
    download_client_type TEXT NOT NULL,
    download_client_item_id TEXT NOT NULL,
    source_title TEXT,
    submitted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    collection_id TEXT,
    tracked_state TEXT,
    tracked_state_at TIMESTAMPTZ,
    source_hint TEXT,
    source_kind TEXT,
    request_signature TEXT,
    episode_id TEXT,
    episode_set_ids TEXT,
    UNIQUE(download_client_id, download_client_type, download_client_item_id)
);

CREATE TABLE IF NOT EXISTS import_artifacts (
    id TEXT PRIMARY KEY,
    source_system TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    import_id TEXT,
    relative_path TEXT,
    normalized_file_name TEXT NOT NULL,
    media_kind TEXT NOT NULL,
    title_id TEXT,
    episode_id TEXT,
    season_number BIGINT,
    episode_number BIGINT,
    result TEXT NOT NULL,
    reason_code TEXT,
    imported_media_file_id TEXT,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS download_queue_commands (
    id TEXT PRIMARY KEY,
    action TEXT NOT NULL,
    client_id TEXT NOT NULL DEFAULT '',
    client_type TEXT NOT NULL,
    download_client_item_id TEXT NOT NULL,
    is_history BOOLEAN NOT NULL DEFAULT FALSE,
    status TEXT NOT NULL,
    error_text TEXT,
    requested_by_user_id TEXT,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_download_queue_commands_active_unique
    ON download_queue_commands(action, client_id, client_type, download_client_item_id, is_history)
    WHERE status IN ('queued', 'running');

CREATE TABLE IF NOT EXISTS imports (
    id TEXT PRIMARY KEY,
    source_system TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    import_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    payload_json JSONB NOT NULL,
    result_json JSONB,
    rename_plan_json JSONB,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS workflow_operations (
    id TEXT PRIMARY KEY,
    operation_type TEXT NOT NULL,
    status TEXT NOT NULL,
    job_key TEXT,
    trigger_source TEXT,
    actor_user_id TEXT,
    progress_json JSONB,
    summary_json JSONB,
    summary_text TEXT,
    error_text TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS domain_events (
    sequence BIGSERIAL PRIMARY KEY,
    event_id TEXT NOT NULL UNIQUE,
    occurred_at TIMESTAMPTZ NOT NULL,
    actor_user_id TEXT,
    title_id TEXT,
    facet TEXT,
    correlation_id TEXT,
    causation_id TEXT,
    schema_version BIGINT NOT NULL,
    stream_kind TEXT NOT NULL,
    stream_id TEXT,
    event_type TEXT NOT NULL,
    payload_json JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS event_subscriber_offsets (
    subscriber_name TEXT PRIMARY KEY,
    sequence BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS job_runs (
    id TEXT PRIMARY KEY,
    job_key TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    progress_json JSONB,
    summary_json JSONB,
    error_text TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS notification_channels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    channel_type TEXT NOT NULL,
    config_json JSONB NOT NULL,
    is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS notification_subscriptions (
    id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL REFERENCES notification_channels(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS external_import_monitor_snapshots (
    facet TEXT PRIMARY KEY,
    payload_json JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS title_search_terms (
    term_id BIGSERIAL PRIMARY KEY,
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    facet TEXT NOT NULL,
    term_kind TEXT NOT NULL,
    raw_term TEXT NOT NULL,
    normalized_term TEXT NOT NULL,
    weight BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_title_search_terms_title
    ON title_search_terms(title_id);

CREATE INDEX IF NOT EXISTS idx_title_search_terms_facet_normalized
    ON title_search_terms(facet, normalized_term);

CREATE UNIQUE INDEX IF NOT EXISTS idx_title_search_terms_unique
    ON title_search_terms(title_id, term_kind, normalized_term);
