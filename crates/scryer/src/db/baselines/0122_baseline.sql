CREATE TABLE blocklist (
    id           TEXT PRIMARY KEY,
    title_id     TEXT NOT NULL,
    source_title TEXT,
    source_hint  TEXT,
    quality      TEXT,
    download_id  TEXT,
    reason       TEXT,
    data_json    TEXT,
    created_at   TEXT NOT NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);
CREATE TABLE collection_external_ids(
    id TEXT PRIMARY KEY NOT NULL,
    title_id TEXT NOT NULL,
    collection_id TEXT NOT NULL,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    provenance TEXT NOT NULL,
    source_scope TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE,
    FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE CASCADE
);
CREATE TABLE collections(
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL,
    collection_type TEXT NOT NULL,
    collection_index TEXT NOT NULL,
    label TEXT,
    ordered_path TEXT,
    first_episode_number TEXT,
    last_episode_number TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT, monitored INTEGER NOT NULL DEFAULT 1, narrative_order TEXT, interstitial_tvdb_id TEXT, interstitial_name TEXT, interstitial_slug TEXT, interstitial_year INTEGER, interstitial_content_status TEXT, interstitial_overview TEXT, interstitial_poster_url TEXT, interstitial_language TEXT, interstitial_runtime_minutes INTEGER, interstitial_sort_title TEXT, interstitial_imdb_id TEXT, interstitial_genres_json TEXT, interstitial_studio TEXT, interstitial_digital_release_date TEXT, interstitial_association_confidence TEXT, interstitial_continuity_status TEXT, interstitial_movie_form TEXT, interstitial_confidence TEXT, interstitial_signal_summary TEXT, special_movies_json TEXT NOT NULL DEFAULT '[]', interstitial_placement TEXT, interstitial_movie_tmdb_id TEXT, interstitial_movie_mal_id TEXT, interstitial_season_episode TEXT, interstitial_movie_anidb_id TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);
CREATE TABLE domain_events(
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    occurred_at TEXT NOT NULL,
    actor_user_id TEXT,
    title_id TEXT,
    facet TEXT,
    correlation_id TEXT,
    causation_id TEXT,
    schema_version INTEGER NOT NULL,
    stream_kind TEXT NOT NULL,
    stream_id TEXT,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL
);
CREATE TABLE download_clients(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    client_type TEXT NOT NULL,
    base_url TEXT,
    config_json TEXT,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'idle',
    last_error TEXT,
    last_seen_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, client_priority INTEGER NOT NULL DEFAULT 0);
CREATE TABLE download_import_artifacts (
    id TEXT PRIMARY KEY,
    source_system TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    import_id TEXT,
    relative_path TEXT,
    normalized_file_name TEXT NOT NULL,
    media_kind TEXT NOT NULL,
    title_id TEXT,
    episode_id TEXT,
    season_number INTEGER,
    episode_number INTEGER,
    result TEXT NOT NULL,
    reason_code TEXT,
    imported_media_file_id TEXT,
    created_at TEXT NOT NULL, source_client_id TEXT,
    FOREIGN KEY (import_id) REFERENCES imports(id) ON DELETE SET NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL,
    FOREIGN KEY (imported_media_file_id) REFERENCES media_files(id) ON DELETE SET NULL
);
CREATE TABLE download_queue_commands (
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
, client_id TEXT);
CREATE TABLE download_submission_episode_links (
    download_client_id TEXT NOT NULL DEFAULT '',
    download_client_type TEXT NOT NULL,
    download_client_item_id TEXT NOT NULL,
    episode_id TEXT NOT NULL,
    PRIMARY KEY (
        download_client_id,
        download_client_type,
        download_client_item_id,
        episode_id
    ),
    FOREIGN KEY (download_client_id, download_client_type, download_client_item_id)
        REFERENCES download_submissions(download_client_id, download_client_type, download_client_item_id)
        ON DELETE CASCADE
);
CREATE TABLE "download_submissions" (
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
CREATE TABLE episode_external_ids(
    id TEXT PRIMARY KEY NOT NULL,
    title_id TEXT NOT NULL,
    episode_id TEXT NOT NULL,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    provenance TEXT NOT NULL,
    source_scope TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE CASCADE
);
CREATE TABLE episodes(
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL,
    collection_id TEXT,
    episode_type TEXT NOT NULL,
    episode_number TEXT,
    season_number TEXT,
    episode_label TEXT,
    title TEXT,
    air_date TEXT,
    duration_seconds INTEGER,
    has_multi_audio INTEGER DEFAULT 0,
    has_subtitle INTEGER DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT, monitored INTEGER NOT NULL DEFAULT 1, overview TEXT, is_filler INTEGER NOT NULL DEFAULT 0, absolute_number TEXT, is_recap INTEGER NOT NULL DEFAULT 0, tvdb_id TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE,
    FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE SET NULL
);
CREATE TABLE event_outboxes(
    id TEXT PRIMARY KEY,
    history_event_id TEXT NOT NULL,
    channel_key TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempt_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    dispatched_at TEXT,
    FOREIGN KEY (history_event_id) REFERENCES history_events(id) ON DELETE CASCADE
);
CREATE TABLE event_subscriber_offsets(
    subscriber_name TEXT PRIMARY KEY,
    sequence INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE external_import_monitor_snapshot_chunks (
    facet TEXT NOT NULL
        CHECK (facet IN ('movie', 'series', 'anime')),
    entry_kind TEXT NOT NULL
        CHECK (entry_kind IN ('movie', 'series')),
    chunk_index INTEGER NOT NULL,
    payload_ndjson TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (facet, entry_kind, chunk_index)
);
CREATE TABLE external_subtitle_probe_cache (
    media_file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    modified_at TEXT,
    language TEXT,
    hearing_impaired INTEGER,
    detection_source_language TEXT NOT NULL,
    detection_source_hi TEXT NOT NULL,
    probe_version INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (media_file_id, file_path)
);
CREATE TABLE file_episode_map(
    file_id TEXT NOT NULL,
    episode_id TEXT NOT NULL,
    is_filler INTEGER DEFAULT 0,
    PRIMARY KEY (file_id, episode_id),
    FOREIGN KEY (file_id) REFERENCES media_files(id) ON DELETE CASCADE,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE CASCADE
);
CREATE TABLE history_events(
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    actor_user_id TEXT,
    title_id TEXT,
    message TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    source TEXT,
    created_at TEXT NOT NULL,
    metadata_json TEXT,
    FOREIGN KEY (actor_user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL
);
CREATE TABLE imports(
    id TEXT PRIMARY KEY,
    source_system TEXT NOT NULL,
    source_ref TEXT NOT NULL,
    import_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    payload_json TEXT NOT NULL,
    result_json TEXT,
    started_at TEXT,
    finished_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, rename_plan_json TEXT, source_client_id TEXT);
CREATE TABLE indexer_api_quotas (
    indexer_id TEXT PRIMARY KEY NOT NULL,
    api_current INTEGER,
    api_max INTEGER,
    grab_current INTEGER,
    grab_max INTEGER,
    queries_today INTEGER NOT NULL DEFAULT 0,
    last_query_at TEXT,
    last_reset_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE indexers(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    base_url TEXT NOT NULL,
    api_key_encrypted TEXT,
    rate_limit_seconds INTEGER,
    rate_limit_burst INTEGER,
    disabled_until TEXT,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    last_health_status TEXT,
    last_error_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, enable_interactive_search INTEGER NOT NULL DEFAULT 1, enable_auto_search INTEGER NOT NULL DEFAULT 1, config_json TEXT, managed_parent_config_id TEXT, managed_child_key TEXT, managed_metadata_json TEXT);
CREATE TABLE libraries (
    id TEXT PRIMARY KEY,
    facet TEXT NOT NULL,
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE library_probe_signatures(
    title_id TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    probe_signature_scheme TEXT,
    probe_signature_value TEXT,
    last_probed_at TEXT,
    last_changed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);
CREATE TABLE library_roots (
    id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL,
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE
);
CREATE TABLE library_scan_unmatched_items (
    id TEXT PRIMARY KEY,
    facet TEXT NOT NULL,
    scan_session_id TEXT NOT NULL,
    scan_root TEXT NOT NULL,
    item_path TEXT NOT NULL,
    display_name TEXT NOT NULL,
    query TEXT NOT NULL,
    year_hint INTEGER,
    reason_code TEXT NOT NULL,
    error_message TEXT,
    search_attempts_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, status TEXT NOT NULL DEFAULT 'pending', title_id TEXT, library_id TEXT);
CREATE TABLE media_files(
    id TEXT PRIMARY KEY,
    title_id TEXT NOT NULL,
    file_path TEXT NOT NULL UNIQUE,
    size_bytes INTEGER NOT NULL,
    quality_id TEXT,
    hash_sha256 TEXT,
    has_multiaudio INTEGER DEFAULT 0,
    scan_status TEXT NOT NULL DEFAULT 'pending',
    scan_error TEXT,
    created_at TEXT NOT NULL, video_codec TEXT, video_width INTEGER, video_height INTEGER, video_bitrate_kbps INTEGER, video_bit_depth INTEGER, video_hdr_format TEXT, audio_codec TEXT, audio_channels INTEGER, duration_seconds INTEGER, container_format TEXT, analysis_json TEXT, video_frame_rate TEXT, video_profile TEXT, audio_bitrate_kbps INTEGER, scene_name TEXT, release_group TEXT, source_type TEXT, resolution TEXT, video_codec_parsed TEXT, audio_codec_parsed TEXT, acquisition_score INTEGER, scoring_log TEXT, indexer_source TEXT, grabbed_release_title TEXT, grabbed_at TEXT, edition TEXT, original_file_path TEXT, release_hash TEXT, num_chapters INTEGER, source_signature_scheme TEXT, source_signature_value TEXT, audio_profile TEXT, audio_channels_parsed TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);
CREATE TABLE notification_channels(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    channel_type TEXT NOT NULL,
    config_json TEXT NOT NULL,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE notification_subscriptions(
    id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (channel_id) REFERENCES notification_channels(id) ON DELETE CASCADE
);
CREATE TABLE pending_releases (
    id TEXT PRIMARY KEY,
    wanted_item_id TEXT NOT NULL,
    title_id TEXT NOT NULL,
    release_title TEXT NOT NULL,
    release_url TEXT,
    release_size_bytes INTEGER,
    release_score INTEGER NOT NULL,
    scoring_log_json TEXT,
    indexer_source TEXT,
    release_guid TEXT,
    added_at TEXT NOT NULL,
    delay_until TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'waiting',
    grabbed_at TEXT
, source_kind TEXT, source_password TEXT, published_at TEXT, info_hash TEXT);
CREATE TABLE plugin_catalog_sources (
    source_key      TEXT PRIMARY KEY,
    source_kind     TEXT NOT NULL,
    source_url      TEXT NOT NULL,
    github_repo     TEXT,
    support_tier    TEXT NOT NULL DEFAULT 'official',
    catalog_json    TEXT,
    last_success_at TEXT,
    last_error      TEXT,
    updated_at      TEXT NOT NULL
);
CREATE TABLE plugin_catalog_status (
    status_key      TEXT PRIMARY KEY,
    status_json     TEXT NOT NULL,
    checked_at      TEXT NOT NULL
);
CREATE TABLE plugin_installations (
    id               TEXT PRIMARY KEY,
    plugin_id        TEXT NOT NULL UNIQUE,
    name             TEXT NOT NULL,
    description      TEXT NOT NULL DEFAULT '',
    version          TEXT NOT NULL,
    sdk_version      TEXT NOT NULL DEFAULT '',
    sdk_constraint   TEXT NOT NULL DEFAULT '',
    scryer_constraint TEXT,
    plugin_type      TEXT NOT NULL DEFAULT 'indexer',
    provider_type    TEXT NOT NULL,
    is_enabled       INTEGER NOT NULL DEFAULT 1,
    is_builtin       INTEGER NOT NULL DEFAULT 0,
    source_kind      TEXT NOT NULL DEFAULT 'downloaded',
    wasm_bytes       BLOB,
    wasm_encoding    TEXT NOT NULL DEFAULT 'identity',
    wasm_digest_algo TEXT,
    source_url       TEXT,
    support_tier     TEXT NOT NULL DEFAULT 'official',
    publisher        TEXT,
    docs_url         TEXT,
    source_repo      TEXT,
    manifest_url     TEXT,
    wasm_digest      TEXT,
    artifact_digest  TEXT,
    installed_at     TEXT NOT NULL,
    updated_at       TEXT NOT NULL
, descriptor_json TEXT);
CREATE TABLE post_processing_script_runs (
    id TEXT PRIMARY KEY,
    script_id TEXT NOT NULL,
    script_name TEXT NOT NULL,                    -- denormalized for history
    title_id TEXT,
    title_name TEXT,
    facet TEXT,
    file_path TEXT,
    status TEXT NOT NULL,                         -- 'success' | 'failed' | 'timeout' | 'running'
    exit_code INTEGER,
    stdout_tail TEXT,                             -- last 4KB
    stderr_tail TEXT,                             -- last 4KB
    duration_ms INTEGER,
    env_payload_json TEXT,                        -- the JSON payload passed to the script
    started_at TEXT NOT NULL,
    completed_at TEXT,
    FOREIGN KEY (script_id) REFERENCES post_processing_scripts(id) ON DELETE CASCADE
);
CREATE TABLE post_processing_scripts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT DEFAULT '',
    script_type TEXT NOT NULL DEFAULT 'inline',   -- 'inline' | 'file'
    script_content TEXT NOT NULL DEFAULT '',       -- shell command (inline) or file path
    applied_facets TEXT NOT NULL DEFAULT '[]',     -- JSON: ["movie","tv","anime"]
    execution_mode TEXT NOT NULL DEFAULT 'blocking', -- 'blocking' | 'fire_and_forget'
    timeout_secs INTEGER DEFAULT 300,
    priority INTEGER NOT NULL DEFAULT 0,          -- lower = runs first
    enabled INTEGER NOT NULL DEFAULT 1,
    debug INTEGER NOT NULL DEFAULT 0,             -- capture stdout/stderr when enabled
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE quality_profile_audio_codec_allowlist(
    profile_id TEXT NOT NULL,
    codec TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, codec),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);
CREATE TABLE quality_profile_audio_codec_blocklist(
    profile_id TEXT NOT NULL,
    codec TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, codec),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);
CREATE TABLE quality_profile_quality_tiers(
    profile_id TEXT NOT NULL,
    quality_tier TEXT NOT NULL,
    sort_order INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, quality_tier),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);
CREATE TABLE quality_profile_source_allowlist(
    profile_id TEXT NOT NULL,
    source TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, source),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);
CREATE TABLE quality_profile_source_blocklist(
    profile_id TEXT NOT NULL,
    source TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, source),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);
CREATE TABLE quality_profile_video_codec_allowlist(
    profile_id TEXT NOT NULL,
    codec TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, codec),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);
CREATE TABLE quality_profile_video_codec_blocklist(
    profile_id TEXT NOT NULL,
    codec TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (profile_id, codec),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);
CREATE TABLE quality_profiles(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    archival_quality TEXT,
    allow_unknown_quality INTEGER NOT NULL DEFAULT 0,
    atmos_preferred INTEGER NOT NULL DEFAULT 0,
    dolby_vision_allowed INTEGER NOT NULL DEFAULT 0,
    detected_hdr_allowed INTEGER NOT NULL DEFAULT 1,
    prefer_remux INTEGER NOT NULL DEFAULT 0,
    allow_bd_disk INTEGER NOT NULL DEFAULT 0,
    allow_upgrades INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
, prefer_dual_audio INTEGER NOT NULL DEFAULT 0, required_audio_languages TEXT NOT NULL DEFAULT '[]', scoring_config TEXT NOT NULL DEFAULT '{}');
CREATE TABLE release_decisions (
    id                  TEXT PRIMARY KEY,
    wanted_item_id      TEXT NOT NULL REFERENCES wanted_items(id) ON DELETE CASCADE,
    title_id            TEXT NOT NULL,
    release_title       TEXT NOT NULL,
    release_url         TEXT,
    release_size_bytes  INTEGER,
    decision_code       TEXT NOT NULL,
    candidate_score     INTEGER NOT NULL,
    current_score       INTEGER,
    score_delta         INTEGER,
    explanation_json    TEXT,
    created_at          TEXT NOT NULL
);
CREATE TABLE release_download_attempts(
    id TEXT PRIMARY KEY,
    title_id TEXT,
    source_hint TEXT,
    source_title TEXT,
    outcome TEXT NOT NULL,
    error_message TEXT,
    attempted_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL, source_password TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL
);
CREATE TABLE rule_set_history (
    id TEXT PRIMARY KEY NOT NULL,
    rule_set_id TEXT NOT NULL,
    action TEXT NOT NULL,
    rego_source TEXT,
    actor_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TABLE rule_sets (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    rego_source TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    priority INTEGER NOT NULL DEFAULT 0,
    applied_facets TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
, is_managed INTEGER NOT NULL DEFAULT 0, managed_key TEXT);
CREATE TABLE settings_definitions(
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL,
    scope TEXT NOT NULL,
    key_name TEXT NOT NULL,
    data_type TEXT NOT NULL,
    default_value_json TEXT,
    is_sensitive INTEGER NOT NULL DEFAULT 0,
    validation_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(category, scope, key_name)
);
CREATE TABLE "settings_values"(
    id TEXT PRIMARY KEY,
    setting_definition_id TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    value_json TEXT NOT NULL,
    source TEXT NOT NULL,
    updated_by_user_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (setting_definition_id) REFERENCES settings_definitions(id) ON DELETE CASCADE
);
CREATE TABLE "subtitle_blocklist" (
    id TEXT PRIMARY KEY,
    media_file_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_file_id TEXT NOT NULL,
    language TEXT NOT NULL,
    reason TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(media_file_id, provider, provider_file_id)
);
CREATE TABLE subtitle_downloads (
    id TEXT PRIMARY KEY,
    media_file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    episode_id TEXT,
    language TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_file_id TEXT,
    file_path TEXT NOT NULL,
    score INTEGER,
    hearing_impaired INTEGER NOT NULL DEFAULT 0,
    forced INTEGER NOT NULL DEFAULT 0,
    ai_translated INTEGER NOT NULL DEFAULT 0,
    machine_translated INTEGER NOT NULL DEFAULT 0,
    uploader TEXT,
    release_info TEXT,
    synced INTEGER NOT NULL DEFAULT 0,
    downloaded_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
, source_kind TEXT NOT NULL DEFAULT 'downloaded');
CREATE TABLE subtitle_provider_configs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    provider_type TEXT NOT NULL,
    config_json TEXT NOT NULL,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    last_health_status TEXT,
    last_error TEXT,
    last_error_at TEXT,
    disabled_until TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, enabled_facets TEXT NOT NULL DEFAULT '[]');
CREATE TABLE title_external_ids(
    id TEXT PRIMARY KEY NOT NULL,
    title_id TEXT NOT NULL,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT, facet TEXT, library_id TEXT,
    FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
);
CREATE TABLE title_image_variants (
  id TEXT PRIMARY KEY,
  title_image_id TEXT NOT NULL,
  variant_key TEXT NOT NULL,
  path TEXT,
  format TEXT NOT NULL,
  width INTEGER NOT NULL,
  height INTEGER NOT NULL,
  bytes BLOB NOT NULL,
  sha256 TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (title_image_id) REFERENCES title_images(id) ON DELETE CASCADE,
  UNIQUE (title_image_id, variant_key)
);
CREATE TABLE title_images (
  id TEXT PRIMARY KEY,
  title_id TEXT NOT NULL,
  provider TEXT NOT NULL,
  provider_image_id TEXT,
  kind TEXT NOT NULL,
  source_url TEXT NOT NULL,
  source_etag TEXT,
  source_last_modified TEXT,
  source_format TEXT NOT NULL,
  source_width INTEGER,
  source_height INTEGER,
  storage_mode TEXT NOT NULL,
  master_path TEXT,
  master_format TEXT NOT NULL,
  master_sha256 TEXT NOT NULL,
  master_width INTEGER NOT NULL,
  master_height INTEGER NOT NULL,
  bytes BLOB NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE,
  UNIQUE (title_id, kind)
);
CREATE VIRTUAL TABLE title_search_spellfix USING spellfix1;
CREATE TABLE title_search_terms (
    term_id INTEGER PRIMARY KEY,
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    facet TEXT NOT NULL,
    term_kind TEXT NOT NULL,
    raw_term TEXT NOT NULL,
    normalized_term TEXT NOT NULL,
    weight INTEGER NOT NULL
);
CREATE TABLE titles(
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    name_normalized TEXT NOT NULL DEFAULT '',
    facet TEXT NOT NULL,
    monitored INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'active',
    tags TEXT NOT NULL DEFAULT '[]',
    external_ids TEXT NOT NULL DEFAULT '[]',
    created_by TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT,
    deleted_at TEXT
, year INTEGER, overview TEXT, poster_url TEXT, sort_title TEXT, slug TEXT, imdb_id TEXT, runtime_minutes INTEGER, genres TEXT NOT NULL DEFAULT '[]', content_status TEXT, language TEXT, first_aired TEXT, network TEXT, studio TEXT, country TEXT, aliases TEXT NOT NULL DEFAULT '[]', metadata_language TEXT, metadata_fetched_at TEXT, min_availability TEXT, digital_release_date TEXT, banner_url TEXT, background_url TEXT, folder_path TEXT, tagged_aliases_json TEXT DEFAULT '[]', poster_local_path TEXT, banner_local_path TEXT, background_local_path TEXT, metadata_hydration_next_attempt_at TEXT, metadata_hydration_attempt_count INTEGER NOT NULL DEFAULT 0, library_id TEXT);
CREATE TABLE user_app_permission_masks (
    user_id TEXT NOT NULL,
    permission_mask INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
CREATE TABLE user_library_permission_masks (
    user_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    permission_mask INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, library_id),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE
);
CREATE TABLE "users" (
    id TEXT PRIMARY KEY NOT NULL,
    username TEXT NOT NULL UNIQUE,
    display_name TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    password_hash TEXT,
    passkey_public_key TEXT,
    locale TEXT,
    created_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT '',
    last_login_at TEXT
);
CREATE TABLE wanted_items (
    id              TEXT PRIMARY KEY,
    title_id        TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    episode_id      TEXT REFERENCES episodes(id) ON DELETE CASCADE,
    media_type      TEXT NOT NULL,
    search_phase    TEXT NOT NULL DEFAULT 'primary',
    next_search_at  TEXT,
    last_search_at  TEXT,
    search_count    INTEGER NOT NULL DEFAULT 0,
    baseline_date   TEXT,
    status          TEXT NOT NULL DEFAULT 'wanted',
    grabbed_release TEXT,
    current_score   INTEGER,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL, collection_id TEXT REFERENCES collections(id),
    UNIQUE(title_id, episode_id)
);
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
CREATE INDEX idx_blocklist_source_title
    ON blocklist (source_title)
    WHERE source_title IS NOT NULL;
CREATE INDEX idx_blocklist_title_id
    ON blocklist (title_id);
CREATE INDEX idx_collection_external_ids_title_provenance
    ON collection_external_ids(title_id, provenance);
CREATE UNIQUE INDEX idx_collection_external_ids_unique
    ON collection_external_ids(collection_id, source, external_id, provenance, source_scope);
CREATE INDEX idx_collections_title
    ON collections (title_id, collection_type);
CREATE INDEX idx_domain_events_event_type_sequence
    ON domain_events (event_type, sequence DESC);
CREATE INDEX idx_domain_events_facet_sequence
    ON domain_events (facet, sequence DESC);
CREATE INDEX idx_domain_events_occurred_at
    ON domain_events (occurred_at DESC);
CREATE INDEX idx_domain_events_stream_sequence
    ON domain_events (stream_kind, stream_id, sequence DESC);
CREATE INDEX idx_domain_events_title_sequence
    ON domain_events (title_id, sequence DESC);
CREATE INDEX idx_download_clients_client_priority
    ON download_clients (client_priority);
CREATE UNIQUE INDEX idx_download_clients_name
    ON download_clients (name);
CREATE INDEX idx_download_import_artifacts_episode
    ON download_import_artifacts (episode_id, result);
CREATE INDEX idx_download_import_artifacts_retention
    ON download_import_artifacts (created_at, import_id);
CREATE INDEX idx_download_import_artifacts_source
    ON download_import_artifacts (COALESCE(source_client_id, ''), source_system, source_ref, created_at);
CREATE UNIQUE INDEX idx_download_queue_commands_active_unique
ON download_queue_commands(action, COALESCE(client_id, ''), client_type, download_client_item_id, is_history)
WHERE status IN ('queued', 'running');
CREATE INDEX idx_download_queue_commands_source
ON download_queue_commands(COALESCE(client_id, ''), client_type, download_client_item_id, is_history, created_at DESC);
CREATE INDEX idx_download_queue_commands_status
ON download_queue_commands(action, status, updated_at);
CREATE INDEX idx_download_submission_episode_links_episode
ON download_submission_episode_links(episode_id);
CREATE INDEX idx_download_submissions_title_request_signature
    ON download_submissions(title_id, request_signature);
CREATE INDEX idx_episode_external_ids_title_provenance
    ON episode_external_ids(title_id, provenance);
CREATE UNIQUE INDEX idx_episode_external_ids_unique
    ON episode_external_ids(episode_id, source, external_id, provenance, source_scope);
CREATE INDEX idx_episodes_collection
    ON episodes (collection_id);
CREATE INDEX idx_episodes_title
    ON episodes (title_id, season_number);
CREATE INDEX idx_event_outboxes_channel
    ON event_outboxes (channel_key);
CREATE INDEX idx_event_outboxes_status
    ON event_outboxes (status, updated_at);
CREATE INDEX idx_external_subtitle_probe_cache_file_path
    ON external_subtitle_probe_cache(file_path);
CREATE INDEX idx_external_subtitle_probe_cache_media_file
    ON external_subtitle_probe_cache(media_file_id);
CREATE INDEX idx_file_episode_map_episode
    ON file_episode_map (episode_id);
CREATE INDEX idx_history_events_occurred_at
    ON history_events (occurred_at DESC);
CREATE INDEX idx_history_events_title_time
    ON history_events (title_id, occurred_at DESC);
CREATE INDEX idx_history_events_type_time
    ON history_events (event_type, occurred_at DESC);
CREATE INDEX idx_history_title_time
    ON history_events (title_id, occurred_at DESC);
CREATE INDEX idx_history_type_time
    ON history_events (event_type, occurred_at DESC);
CREATE UNIQUE INDEX idx_imports_source_ref
    ON imports (COALESCE(source_client_id, ''), source_system, source_ref, import_type);
CREATE INDEX idx_imports_status_updated_at
    ON imports (status, updated_at);
CREATE UNIQUE INDEX idx_indexers_managed_child_identity
ON indexers(managed_parent_config_id, managed_child_key)
WHERE managed_parent_config_id IS NOT NULL AND managed_child_key IS NOT NULL;
CREATE INDEX idx_indexers_managed_parent ON indexers(managed_parent_config_id);
CREATE UNIQUE INDEX idx_libraries_facet_slug
    ON libraries(facet, slug);
CREATE INDEX idx_library_probe_signatures_last_probed
    ON library_probe_signatures (last_probed_at DESC);
CREATE INDEX idx_library_roots_library
    ON library_roots(library_id, is_default DESC, path ASC);
CREATE UNIQUE INDEX idx_library_roots_normalized_path
    ON library_roots(normalized_path);
CREATE INDEX idx_library_scan_unmatched_items_facet_status_updated
    ON library_scan_unmatched_items (facet, status, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_facet_title_status_updated
    ON library_scan_unmatched_items (facet, title_id, status, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_facet_updated
    ON library_scan_unmatched_items (facet, updated_at DESC);
CREATE UNIQUE INDEX idx_library_scan_unmatched_items_library_path
    ON library_scan_unmatched_items(library_id, item_path);
CREATE INDEX idx_library_scan_unmatched_items_library_updated
    ON library_scan_unmatched_items(library_id, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_root_status_updated
    ON library_scan_unmatched_items (facet, scan_root, status, updated_at DESC);
CREATE INDEX idx_library_scan_unmatched_items_root_updated
    ON library_scan_unmatched_items (facet, scan_root, updated_at DESC);
CREATE INDEX idx_media_files_title
    ON media_files (title_id);
CREATE INDEX idx_media_files_title_path
    ON media_files (title_id, file_path);
CREATE UNIQUE INDEX idx_notification_channels_name_type
    ON notification_channels (name, channel_type);
CREATE UNIQUE INDEX idx_notification_subscriptions_channel_scope
    ON notification_subscriptions (channel_id, event_type, COALESCE(scope, ''), COALESCE(scope_id, ''));
CREATE INDEX idx_operations_status_time
    ON workflow_operations (status, started_at DESC);
CREATE INDEX idx_pending_releases_status ON pending_releases(status);
CREATE INDEX idx_pending_releases_wanted ON pending_releases(wanted_item_id, status);
CREATE INDEX idx_plugin_catalog_sources_kind
    ON plugin_catalog_sources(source_kind);
CREATE INDEX idx_pp_script_runs_script_id ON post_processing_script_runs(script_id, started_at DESC);
CREATE INDEX idx_pp_script_runs_title_id ON post_processing_script_runs(title_id, started_at DESC);
CREATE INDEX idx_quality_profile_audio_codec_allowlist_profile
    ON quality_profile_audio_codec_allowlist (profile_id);
CREATE INDEX idx_quality_profile_audio_codec_blocklist_profile
    ON quality_profile_audio_codec_blocklist (profile_id);
CREATE INDEX idx_quality_profile_quality_tiers_profile
    ON quality_profile_quality_tiers (profile_id, sort_order);
CREATE INDEX idx_quality_profile_source_allowlist_profile
    ON quality_profile_source_allowlist (profile_id);
CREATE INDEX idx_quality_profile_source_blocklist_profile
    ON quality_profile_source_blocklist (profile_id);
CREATE INDEX idx_quality_profile_video_codec_allowlist_profile
    ON quality_profile_video_codec_allowlist (profile_id);
CREATE INDEX idx_quality_profile_video_codec_blocklist_profile
    ON quality_profile_video_codec_blocklist (profile_id);
CREATE INDEX idx_quality_profiles_scope
    ON quality_profiles (scope, scope_id);
CREATE INDEX idx_release_decisions_created_at
    ON release_decisions (created_at DESC);
CREATE INDEX idx_release_decisions_wanted
    ON release_decisions(wanted_item_id, created_at DESC);
CREATE INDEX idx_release_download_attempts_outcome_attempted
    ON release_download_attempts (outcome, attempted_at DESC);
CREATE INDEX idx_release_download_attempts_source_hint
    ON release_download_attempts (source_hint);
CREATE INDEX idx_release_download_attempts_source_title
    ON release_download_attempts (source_title);
CREATE INDEX idx_rule_set_history_created_at
    ON rule_set_history (created_at DESC);
CREATE UNIQUE INDEX idx_rule_sets_managed_key ON rule_sets(managed_key) WHERE managed_key IS NOT NULL;
CREATE UNIQUE INDEX idx_setting_values_scope_name
    ON settings_values(setting_definition_id, scope, COALESCE(scope_id, ''));
CREATE INDEX idx_settings_values_definition
    ON settings_values(setting_definition_id);
CREATE INDEX idx_subtitle_blocklist_media_file
    ON subtitle_blocklist(media_file_id);
CREATE INDEX idx_subtitle_downloads_language ON subtitle_downloads(language);
CREATE INDEX idx_subtitle_downloads_media_file ON subtitle_downloads(media_file_id);
CREATE INDEX idx_subtitle_downloads_title ON subtitle_downloads(title_id);
CREATE INDEX idx_subtitle_provider_configs_disabled_until
    ON subtitle_provider_configs(disabled_until);
CREATE INDEX idx_subtitle_provider_configs_enabled
    ON subtitle_provider_configs(is_enabled);
CREATE INDEX idx_subtitle_provider_configs_provider_type
    ON subtitle_provider_configs(provider_type);
CREATE UNIQUE INDEX idx_title_external_ids_library_lookup
    ON title_external_ids(library_id, source, external_id);
CREATE INDEX idx_title_external_ids_title_id
    ON title_external_ids(title_id);
CREATE INDEX idx_title_image_variants_image_variant
  ON title_image_variants(title_image_id, variant_key);
CREATE INDEX idx_title_images_title_kind ON title_images(title_id, kind);
CREATE INDEX idx_title_search_terms_facet_normalized_term
    ON title_search_terms(facet, normalized_term);
CREATE INDEX idx_title_search_terms_normalized_term
    ON title_search_terms(normalized_term);
CREATE INDEX idx_title_search_terms_title_id
    ON title_search_terms(title_id);
CREATE UNIQUE INDEX idx_title_search_terms_title_kind_normalized
    ON title_search_terms(title_id, term_kind, normalized_term);
CREATE INDEX idx_titles_facet_monitored
    ON titles (facet, monitored);
CREATE INDEX idx_titles_facet_normalized_slug
ON titles (facet, LOWER(TRIM(slug)))
WHERE slug IS NOT NULL AND TRIM(slug) <> '';
CREATE INDEX idx_titles_library_name
    ON titles(library_id, LOWER(name), id);
CREATE INDEX idx_titles_metadata_hydration_due
    ON titles(metadata_hydration_next_attempt_at, metadata_fetched_at);
CREATE UNIQUE INDEX idx_wanted_items_collection_id ON wanted_items(collection_id) WHERE collection_id IS NOT NULL;
CREATE UNIQUE INDEX idx_wanted_items_movie_unique
    ON wanted_items(title_id)
    WHERE episode_id IS NULL AND collection_id IS NULL;
CREATE INDEX idx_wanted_items_next_search
    ON wanted_items(status, next_search_at);
CREATE INDEX idx_wanted_items_title
    ON wanted_items(title_id);
CREATE INDEX idx_workflow_operations_job_key_started
    ON workflow_operations (job_key, started_at DESC);
CREATE INDEX idx_workflow_operations_job_key_status
    ON workflow_operations (job_key, status, started_at DESC);
CREATE INDEX idx_workflow_operations_status_started
    ON workflow_operations (status, started_at);
INSERT INTO "libraries" ("id", "facet", "name", "slug", "is_default", "created_at", "updated_at") VALUES ('anime_default_library', 'anime', 'Anime', 'anime', 1, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO "libraries" ("id", "facet", "name", "slug", "is_default", "created_at", "updated_at") VALUES ('movie_default_library', 'movie', 'Movies', 'movies', 1, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO "libraries" ("id", "facet", "name", "slug", "is_default", "created_at", "updated_at") VALUES ('series_default_library', 'series', 'Series', 'series', 1, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO "library_roots" ("id", "library_id", "path", "normalized_path", "is_default", "created_at", "updated_at") VALUES ('canonical_root_for_anime_default_library', 'anime_default_library', '/data/anime', '/data/anime', 1, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO "library_roots" ("id", "library_id", "path", "normalized_path", "is_default", "created_at", "updated_at") VALUES ('canonical_root_for_movie_default_library', 'movie_default_library', '/data/movies', '/data/movies', 1, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO "library_roots" ("id", "library_id", "path", "normalized_path", "is_default", "created_at", "updated_at") VALUES ('canonical_root_for_series_default_library', 'series_default_library', '/data/series', '/data/series', 1, '1970-01-01T00:00:00Z', '1970-01-01T00:00:00Z');
INSERT INTO "quality_profile_quality_tiers" ("profile_id", "quality_tier", "sort_order", "created_at") VALUES ('1080p', '1080P', 0, '1970-01-01T00:00:00Z');
INSERT INTO "quality_profile_quality_tiers" ("profile_id", "quality_tier", "sort_order", "created_at") VALUES ('1080p', '720P', 1, '1970-01-01T00:00:00Z');
INSERT INTO "quality_profile_quality_tiers" ("profile_id", "quality_tier", "sort_order", "created_at") VALUES ('4k', '1080P', 1, '1970-01-01T00:00:00Z');
INSERT INTO "quality_profile_quality_tiers" ("profile_id", "quality_tier", "sort_order", "created_at") VALUES ('4k', '2160P', 0, '1970-01-01T00:00:00Z');
INSERT INTO "quality_profile_quality_tiers" ("profile_id", "quality_tier", "sort_order", "created_at") VALUES ('4k', '720P', 2, '1970-01-01T00:00:00Z');
INSERT INTO "quality_profiles" ("id", "name", "scope", "scope_id", "archival_quality", "allow_unknown_quality", "atmos_preferred", "dolby_vision_allowed", "detected_hdr_allowed", "prefer_remux", "allow_bd_disk", "allow_upgrades", "created_at", "prefer_dual_audio", "required_audio_languages", "scoring_config") VALUES ('1080p', '1080P', 'system', NULL, '1080P', 0, 1, 1, 1, 1, 0, 1, '1970-01-01T00:00:00Z', 0, '[]', '{}');
INSERT INTO "quality_profiles" ("id", "name", "scope", "scope_id", "archival_quality", "allow_unknown_quality", "atmos_preferred", "dolby_vision_allowed", "detected_hdr_allowed", "prefer_remux", "allow_bd_disk", "allow_upgrades", "created_at", "prefer_dual_audio", "required_audio_languages", "scoring_config") VALUES ('4k', '4K', 'system', NULL, '2160P', 0, 1, 1, 1, 1, 0, 1, '1970-01-01T00:00:00Z', 0, '[]', '{}');
INSERT INTO "users" ("id", "username", "display_name", "status", "password_hash", "passkey_public_key", "locale", "created_at", "updated_at", "last_login_at") VALUES ('00000000000000000000000000000001', 'admin', NULL, 'active', NULL, NULL, NULL, '', '1970-01-01T00:00:00Z', NULL);
