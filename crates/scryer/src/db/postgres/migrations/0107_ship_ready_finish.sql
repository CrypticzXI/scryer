CREATE TABLE IF NOT EXISTS collection_external_ids (
    id TEXT PRIMARY KEY,
    collection_id TEXT,
    source TEXT,
    external_id TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ
);
ALTER TABLE collection_external_ids ADD COLUMN IF NOT EXISTS provenance TEXT DEFAULT 'metadata';
ALTER TABLE collection_external_ids ADD COLUMN IF NOT EXISTS source_scope TEXT;

CREATE TABLE IF NOT EXISTS download_import_artifacts (
    id TEXT PRIMARY KEY,
    source_system TEXT,
    source_ref TEXT,
    import_id TEXT,
    relative_path TEXT,
    normalized_file_name TEXT,
    media_kind TEXT,
    title_id TEXT,
    episode_id TEXT,
    season_number BIGINT,
    episode_number BIGINT,
    result TEXT,
    reason_code TEXT,
    imported_media_file_id TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS download_jobs (
    id TEXT PRIMARY KEY,
    job_key TEXT,
    status TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    progress_json JSONB,
    summary_json JSONB,
    error_text TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS download_submission_episode_links (
    submission_id TEXT,
    episode_id TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (submission_id, episode_id)
);

CREATE TABLE IF NOT EXISTS entitlements (
    id TEXT PRIMARY KEY,
    key TEXT,
    name TEXT,
    description TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS episode_external_ids (
    id TEXT PRIMARY KEY,
    episode_id TEXT,
    source TEXT,
    external_id TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ
);
ALTER TABLE episode_external_ids ADD COLUMN IF NOT EXISTS provenance TEXT DEFAULT 'metadata';
ALTER TABLE episode_external_ids ADD COLUMN IF NOT EXISTS source_scope TEXT;

ALTER TABLE collections ADD COLUMN IF NOT EXISTS narrative_order TEXT;
ALTER TABLE collections ADD COLUMN IF NOT EXISTS first_episode_number TEXT;
ALTER TABLE collections ADD COLUMN IF NOT EXISTS last_episode_number TEXT;
ALTER TABLE collections ADD COLUMN IF NOT EXISTS interstitial_movie_json JSONB;
ALTER TABLE collections ADD COLUMN IF NOT EXISTS specials_movies_json JSONB DEFAULT '[]'::JSONB;
ALTER TABLE collections ADD COLUMN IF NOT EXISTS interstitial_season_episode TEXT;
ALTER TABLE collections ADD COLUMN IF NOT EXISTS monitored BOOLEAN DEFAULT TRUE;

ALTER TABLE episodes ADD COLUMN IF NOT EXISTS has_multi_audio BOOLEAN DEFAULT FALSE;
ALTER TABLE episodes ADD COLUMN IF NOT EXISTS has_subtitle BOOLEAN DEFAULT FALSE;
ALTER TABLE episodes ADD COLUMN IF NOT EXISTS is_filler BOOLEAN DEFAULT FALSE;
ALTER TABLE episodes ADD COLUMN IF NOT EXISTS is_recap BOOLEAN DEFAULT FALSE;
ALTER TABLE episodes ADD COLUMN IF NOT EXISTS absolute_number TEXT;
ALTER TABLE episodes ADD COLUMN IF NOT EXISTS overview TEXT;
ALTER TABLE episodes ADD COLUMN IF NOT EXISTS tvdb_id TEXT;

ALTER TABLE titles ADD COLUMN IF NOT EXISTS metadata_hydration_next_attempt_at TIMESTAMPTZ;
ALTER TABLE titles ADD COLUMN IF NOT EXISTS metadata_hydration_attempt_count BIGINT DEFAULT 0;

CREATE TABLE IF NOT EXISTS event_outboxes (
    id TEXT PRIMARY KEY,
    event_id TEXT,
    subscriber TEXT,
    payload_json JSONB,
    status TEXT,
    dispatched_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS external_subtitle_probe_cache (
    media_file_id TEXT,
    file_path TEXT,
    size_bytes BIGINT,
    modified_at TIMESTAMPTZ,
    language TEXT,
    hearing_impaired BOOLEAN,
    detection_source_language TEXT,
    detection_source_hi TEXT,
    probe_version BIGINT,
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (media_file_id, file_path)
);

CREATE TABLE IF NOT EXISTS file_episode_map (
    file_id TEXT,
    episode_id TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (file_id, episode_id)
);

CREATE TABLE IF NOT EXISTS history_events (
    id TEXT PRIMARY KEY,
    title_id TEXT,
    event_type TEXT,
    event_json JSONB,
    occurred_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS indexer_api_quotas (
    id TEXT PRIMARY KEY,
    indexer_id TEXT,
    quota_key TEXT,
    used_count BIGINT,
    reset_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS integration_tokens (
    id TEXT PRIMARY KEY,
    integration_key TEXT,
    token_name TEXT,
    token_secret TEXT,
    metadata_json JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS libraries (
    id TEXT PRIMARY KEY,
    name TEXT,
    slug TEXT,
    facet TEXT,
    root_path TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS library_probe_signatures (
    title_id TEXT PRIMARY KEY,
    path TEXT,
    probe_signature_scheme TEXT,
    probe_signature_value TEXT,
    last_probed_at TIMESTAMPTZ,
    last_changed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

ALTER TABLE library_scan_unmatched_items ADD COLUMN IF NOT EXISTS scan_session_id TEXT;
ALTER TABLE library_scan_unmatched_items ADD COLUMN IF NOT EXISTS display_name TEXT;
ALTER TABLE library_scan_unmatched_items ADD COLUMN IF NOT EXISTS query TEXT;
ALTER TABLE library_scan_unmatched_items ADD COLUMN IF NOT EXISTS year_hint BIGINT;
ALTER TABLE library_scan_unmatched_items ADD COLUMN IF NOT EXISTS reason_code TEXT;
ALTER TABLE library_scan_unmatched_items ADD COLUMN IF NOT EXISTS error_message TEXT;
ALTER TABLE library_scan_unmatched_items ADD COLUMN IF NOT EXISTS search_attempts_json JSONB DEFAULT '[]'::JSONB;

UPDATE library_scan_unmatched_items
SET scan_session_id = COALESCE(scan_session_id, ''),
    display_name = COALESCE(display_name, item_path),
    query = COALESCE(query, item_path),
    reason_code = COALESCE(reason_code, 'unknown'),
    search_attempts_json = COALESCE(search_attempts_json, '[]'::JSONB);

CREATE TABLE IF NOT EXISTS library_roots (
    id TEXT PRIMARY KEY,
    library_id TEXT,
    path TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS media_files (
    id TEXT PRIMARY KEY,
    title_id TEXT,
    episode_id TEXT,
    file_path TEXT UNIQUE,
    size_bytes BIGINT,
    quality_id TEXT,
    hash_sha256 TEXT,
    has_multiaudio BOOLEAN DEFAULT FALSE,
    scan_status TEXT DEFAULT 'pending',
    scan_error TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    video_codec TEXT,
    video_width BIGINT,
    video_height BIGINT,
    video_bitrate_kbps BIGINT,
    video_bit_depth BIGINT,
    video_hdr_format TEXT,
    audio_codec TEXT,
    audio_channels BIGINT,
    duration_seconds BIGINT,
    container_format TEXT,
    analysis_json JSONB,
    video_frame_rate TEXT,
    video_profile TEXT,
    audio_bitrate_kbps BIGINT,
    scene_name TEXT,
    release_group TEXT,
    source_type TEXT,
    resolution TEXT,
    video_codec_parsed TEXT,
    audio_codec_parsed TEXT,
    acquisition_score BIGINT,
    scoring_log TEXT,
    indexer_source TEXT,
    grabbed_release_title TEXT,
    grabbed_at TIMESTAMPTZ,
    edition TEXT,
    original_file_path TEXT,
    release_hash TEXT,
    num_chapters BIGINT,
    source_signature_scheme TEXT,
    source_signature_value TEXT,
    audio_profile TEXT,
    audio_channels_parsed TEXT
);

CREATE TABLE IF NOT EXISTS pending_releases (
    id TEXT PRIMARY KEY,
    wanted_item_id TEXT,
    title_id TEXT,
    release_title TEXT,
    release_url TEXT,
    source_kind TEXT,
    release_size_bytes BIGINT,
    release_score BIGINT,
    scoring_log_json JSONB,
    indexer_source TEXT,
    release_guid TEXT,
    added_at TIMESTAMPTZ,
    delay_until TIMESTAMPTZ,
    status TEXT DEFAULT 'waiting',
    grabbed_at TIMESTAMPTZ,
    source_password TEXT,
    published_at TIMESTAMPTZ,
    info_hash TEXT
);

CREATE TABLE IF NOT EXISTS post_processing_scripts (
    id TEXT PRIMARY KEY,
    name TEXT,
    script_path TEXT,
    is_enabled BOOLEAN DEFAULT TRUE,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);
ALTER TABLE post_processing_scripts ADD COLUMN IF NOT EXISTS record_json JSONB;
ALTER TABLE post_processing_scripts ADD COLUMN IF NOT EXISTS priority BIGINT DEFAULT 0;

CREATE TABLE IF NOT EXISTS post_processing_script_runs (
    id TEXT PRIMARY KEY,
    script_id TEXT,
    status TEXT,
    output_text TEXT,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
ALTER TABLE post_processing_script_runs ADD COLUMN IF NOT EXISTS record_json JSONB;

CREATE TABLE IF NOT EXISTS push_subscriptions (
    id TEXT PRIMARY KEY,
    endpoint TEXT,
    p256dh TEXT,
    auth TEXT,
    user_id TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS quality_profiles (
    id TEXT PRIMARY KEY,
    name TEXT,
    scope TEXT,
    scope_id TEXT,
    archival_quality TEXT,
    allow_unknown_quality BOOLEAN DEFAULT FALSE,
    atmos_preferred BOOLEAN DEFAULT FALSE,
    dolby_vision_allowed BOOLEAN DEFAULT FALSE,
    detected_hdr_allowed BOOLEAN DEFAULT TRUE,
    prefer_remux BOOLEAN DEFAULT FALSE,
    allow_bd_disk BOOLEAN DEFAULT FALSE,
    allow_upgrades BOOLEAN DEFAULT TRUE,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    prefer_dual_audio BOOLEAN DEFAULT FALSE,
    required_audio_languages JSONB DEFAULT '[]'::JSONB,
    scoring_config JSONB DEFAULT '{}'::JSONB
);

CREATE TABLE IF NOT EXISTS quality_profile_source_allowlist (
    profile_id TEXT,
    source TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (profile_id, source)
);

CREATE TABLE IF NOT EXISTS quality_profile_source_blocklist (
    profile_id TEXT,
    source TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (profile_id, source)
);

CREATE TABLE IF NOT EXISTS quality_profile_video_codec_allowlist (
    profile_id TEXT,
    codec TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (profile_id, codec)
);

CREATE TABLE IF NOT EXISTS quality_profile_video_codec_blocklist (
    profile_id TEXT,
    codec TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (profile_id, codec)
);

CREATE TABLE IF NOT EXISTS quality_profile_audio_codec_allowlist (
    profile_id TEXT,
    codec TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (profile_id, codec)
);

CREATE TABLE IF NOT EXISTS quality_profile_audio_codec_blocklist (
    profile_id TEXT,
    codec TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (profile_id, codec)
);

CREATE TABLE IF NOT EXISTS quality_profile_quality_tiers (
    profile_id TEXT,
    quality TEXT,
    tier_rank BIGINT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (profile_id, quality)
);

CREATE TABLE IF NOT EXISTS quarantine_items (
    id TEXT PRIMARY KEY,
    media_file_id TEXT,
    file_path TEXT,
    reason_code TEXT,
    reason_json JSONB,
    quarantined_by TEXT,
    quarantined_at TIMESTAMPTZ,
    release_id TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS release_decisions (
    id TEXT PRIMARY KEY,
    wanted_item_id TEXT,
    title_id TEXT,
    release_title TEXT,
    release_url TEXT,
    release_size_bytes BIGINT,
    decision_code TEXT,
    candidate_score BIGINT,
    current_score BIGINT,
    score_delta BIGINT,
    explanation_json JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS releases (
    id TEXT PRIMARY KEY,
    title_id TEXT,
    collection_id TEXT,
    release_scope TEXT,
    indexer_id TEXT,
    release_title TEXT,
    release_url TEXT,
    release_size_bytes BIGINT,
    release_score BIGINT,
    payload_json JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS rule_sets (
    id TEXT PRIMARY KEY,
    name TEXT,
    managed_key TEXT,
    rule_json JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);
ALTER TABLE rule_sets ADD COLUMN IF NOT EXISTS record_json JSONB;
ALTER TABLE rule_sets ADD COLUMN IF NOT EXISTS enabled BOOLEAN DEFAULT TRUE;
ALTER TABLE rule_sets ADD COLUMN IF NOT EXISTS priority BIGINT DEFAULT 0;
ALTER TABLE rule_sets ADD COLUMN IF NOT EXISTS is_managed BOOLEAN DEFAULT FALSE;

CREATE TABLE IF NOT EXISTS rule_set_history (
    id TEXT PRIMARY KEY,
    rule_set_id TEXT,
    event_json JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS scheduler_jobs (
    id TEXT PRIMARY KEY,
    job_name TEXT,
    status TEXT,
    next_run_at TIMESTAMPTZ,
    payload_json JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS subtitle_downloads (
    id TEXT PRIMARY KEY,
    media_file_id TEXT,
    title_id TEXT,
    episode_id TEXT,
    language TEXT,
    provider TEXT,
    provider_file_id TEXT,
    file_path TEXT,
    score BIGINT,
    hearing_impaired BOOLEAN DEFAULT FALSE,
    forced BOOLEAN DEFAULT FALSE,
    ai_translated BOOLEAN DEFAULT FALSE,
    machine_translated BOOLEAN DEFAULT FALSE,
    uploader TEXT,
    release_info TEXT,
    synced BOOLEAN DEFAULT FALSE,
    downloaded_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    source_kind TEXT DEFAULT 'downloaded'
);

CREATE TABLE IF NOT EXISTS subtitle_provider_configs (
    id TEXT PRIMARY KEY,
    name TEXT,
    provider_type TEXT,
    config_json JSONB,
    record_json JSONB DEFAULT '{}'::JSONB,
    is_enabled BOOLEAN DEFAULT TRUE,
    last_health_status TEXT,
    last_error TEXT,
    last_error_at TIMESTAMPTZ,
    disabled_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    enabled_facets JSONB DEFAULT '[]'::JSONB
);

CREATE TABLE IF NOT EXISTS subtitle_blocklist (
    id TEXT,
    media_file_id TEXT,
    provider TEXT,
    provider_file_id TEXT,
    language TEXT,
    reason TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (media_file_id, provider, provider_file_id)
);

CREATE TABLE IF NOT EXISTS title_aliases (
    id TEXT PRIMARY KEY,
    title_id TEXT,
    alias_type TEXT,
    alias_value TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
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

CREATE TABLE IF NOT EXISTS upgrades (
    id TEXT PRIMARY KEY,
    component TEXT,
    from_version TEXT,
    to_version TEXT,
    status TEXT DEFAULT 'pending',
    workflow_operation_id TEXT,
    actor_user_id TEXT,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    error_message TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS user_app_permission_masks (
    user_id TEXT,
    mask BIGINT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (user_id)
);

CREATE TABLE IF NOT EXISTS user_entitlements (
    user_id TEXT,
    entitlement_id TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (user_id, entitlement_id)
);

CREATE TABLE IF NOT EXISTS user_library_permission_masks (
    user_id TEXT,
    library_id TEXT,
    mask BIGINT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (user_id, library_id)
);

CREATE TABLE IF NOT EXISTS wanted_items (
    id TEXT PRIMARY KEY,
    title_id TEXT,
    episode_id TEXT,
    collection_id TEXT,
    media_type TEXT,
    search_phase TEXT DEFAULT 'primary',
    next_search_at TIMESTAMPTZ,
    last_search_at TIMESTAMPTZ,
    search_count BIGINT DEFAULT 0,
    baseline_date TIMESTAMPTZ,
    status TEXT DEFAULT 'wanted',
    grabbed_release TEXT,
    current_score BIGINT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_wanted_items_collection_id
    ON wanted_items(collection_id) WHERE collection_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_wanted_items_episode_unique
    ON wanted_items(title_id, episode_id) WHERE episode_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_wanted_items_movie_unique
    ON wanted_items(title_id) WHERE episode_id IS NULL AND collection_id IS NULL;
