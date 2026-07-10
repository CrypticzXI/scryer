CREATE TABLE IF NOT EXISTS discovery_sync_runs (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    trigger_source TEXT NOT NULL,
    region TEXT NOT NULL,
    language TEXT NOT NULL,
    subject_count BIGINT NOT NULL DEFAULT 0,
    subject_fingerprint TEXT,
    previous_subject_fingerprint TEXT,
    base_generation_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    changed_subject_count BIGINT NOT NULL DEFAULT 0,
    affected_target_count BIGINT NOT NULL DEFAULT 0,
    smg_request_id TEXT,
    smg_status TEXT,
    discovery_index_watermark TEXT,
    page_count INTEGER,
    item_count BIGINT,
    facet_count BIGINT,
    raw_submit_json JSONB,
    raw_changes_json JSONB,
    raw_final_status_json JSONB,
    raw_ack_json JSONB,
    error_text TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS discovery_sync_state (
    scope_key TEXT PRIMARY KEY NOT NULL,
    last_success_generation_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    last_public_feed_generation_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    last_subject_fingerprint TEXT,
    last_context_snapshot_completed_at TIMESTAMPTZ,
    last_incremental_reload_completed_at TIMESTAMPTZ,
    last_public_feed_completed_at TIMESTAMPTZ,
    dirty_since TIMESTAMPTZ,
    dirty_reason_mask BIGINT NOT NULL DEFAULT 0,
    bootstrap_started_at TIMESTAMPTZ,
    bootstrap_quiet_until TIMESTAMPTZ,
    next_context_snapshot_eligible_at TIMESTAMPTZ,
    next_incremental_reload_eligible_at TIMESTAMPTZ,
    next_public_feed_eligible_at TIMESTAMPTZ,
    backoff_until TIMESTAMPTZ,
    startup_jitter_seconds BIGINT NOT NULL DEFAULT 0,
    context_jitter_seconds BIGINT NOT NULL DEFAULT 0,
    incremental_reload_jitter_seconds BIGINT NOT NULL DEFAULT 0,
    public_feed_jitter_seconds BIGINT NOT NULL DEFAULT 0,
    last_seen_domain_event_sequence BIGINT,
    inflight_subject_fingerprint TEXT,
    inflight_domain_event_sequence BIGINT,
    inflight_context_snapshot_run_id TEXT,
    lease_owner_id TEXT,
    lease_expires_at TIMESTAMPTZ,
    transient_failure_count BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS discovery_raw_pages (
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    payload_kind TEXT NOT NULL,
    page_number INTEGER NOT NULL DEFAULT 0,
    compression TEXT NOT NULL DEFAULT 'none',
    raw_payload TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (run_id, payload_kind, page_number)
);

CREATE TABLE IF NOT EXISTS discovery_submitted_subjects (
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    subject_key TEXT NOT NULL,
    title_id TEXT REFERENCES titles(id) ON DELETE SET NULL,
    library_id TEXT,
    library_facet TEXT,
    title_kind TEXT,
    display_title TEXT,
    external_ids_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    raw_subject_json JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_discovery_submitted_subjects_run_key
    ON discovery_submitted_subjects (run_id, subject_key, library_id, title_id);

CREATE TABLE IF NOT EXISTS discovery_pending_context_changes (
    id TEXT PRIMARY KEY NOT NULL,
    scope_key TEXT NOT NULL DEFAULT 'default',
    subject_key TEXT,
    previous_subject_key TEXT,
    change_type TEXT NOT NULL,
    title_id TEXT REFERENCES titles(id) ON DELETE SET NULL,
    previous_title_id TEXT,
    library_facet TEXT,
    raw_subject_json JSONB,
    raw_previous_subject_json JSONB,
    first_seen_sequence BIGINT,
    last_seen_sequence BIGINT,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS discovery_sections (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    section_id TEXT NOT NULL,
    section_type TEXT NOT NULL,
    surface TEXT NOT NULL,
    title TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS canonical_media_subjects (
    id TEXT PRIMARY KEY NOT NULL,
    subject_key TEXT NOT NULL,
    subject_key_norm TEXT NOT NULL,
    language TEXT NOT NULL,
    target_kind TEXT NOT NULL DEFAULT '',
    title_id TEXT REFERENCES titles(id) ON DELETE SET NULL,
    display_title TEXT NOT NULL DEFAULT '',
    year INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (subject_key_norm, language)
);

CREATE TABLE IF NOT EXISTS canonical_media_tags (
    subject_id TEXT NOT NULL REFERENCES canonical_media_subjects(id) ON DELETE CASCADE,
    tag_key TEXT NOT NULL,
    category TEXT NOT NULL,
    name TEXT NOT NULL,
    confidence DOUBLE PRECISION,
    is_adult BOOLEAN NOT NULL DEFAULT FALSE,
    is_spoiler BOOLEAN NOT NULL DEFAULT FALSE,
    sort_index INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (subject_id, tag_key)
);

CREATE TABLE IF NOT EXISTS canonical_media_tag_sources (
    subject_id TEXT NOT NULL,
    tag_key TEXT NOT NULL,
    source TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (subject_id, tag_key)
        REFERENCES canonical_media_tags(subject_id, tag_key) ON DELETE CASCADE,
    UNIQUE (subject_id, tag_key, source)
);

CREATE TABLE IF NOT EXISTS canonical_media_tag_source_keys (
    subject_id TEXT NOT NULL,
    tag_key TEXT NOT NULL,
    source_tag_key TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (subject_id, tag_key)
        REFERENCES canonical_media_tags(subject_id, tag_key) ON DELETE CASCADE,
    UNIQUE (subject_id, tag_key, source_tag_key)
);

CREATE TABLE IF NOT EXISTS discovery_titles (
    id TEXT PRIMARY KEY NOT NULL,
    target_key TEXT NOT NULL,
    target_key_norm TEXT NOT NULL,
    language TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    resolved BOOLEAN NOT NULL DEFAULT FALSE,
    resolved_title_id TEXT REFERENCES titles(id) ON DELETE SET NULL,
    canonical_subject_id TEXT REFERENCES canonical_media_subjects(id) ON DELETE SET NULL,
    display_title TEXT NOT NULL,
    original_title TEXT,
    sort_title TEXT,
    year INTEGER,
    poster_path TEXT,
    poster_url TEXT,
    background_url TEXT,
    overview TEXT,
    content_type TEXT,
    tmdb_collection_id TEXT,
    tmdb_collection_name TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (target_key_norm, language)
);

CREATE TABLE IF NOT EXISTS discovery_title_terms (
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    term_kind TEXT NOT NULL,
    term_category TEXT NOT NULL DEFAULT '',
    term_value TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    UNIQUE (discovery_title_id, term_kind, term_category, term_value)
);

CREATE TABLE IF NOT EXISTS discovery_title_source_tags (
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    category TEXT NOT NULL DEFAULT '',
    name TEXT NOT NULL DEFAULT '',
    sort_index INTEGER NOT NULL DEFAULT 0,
    UNIQUE (discovery_title_id, sort_index, category, name)
);

CREATE TABLE IF NOT EXISTS discovery_title_source_tag_values (
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    source_tag_sort_index INTEGER NOT NULL,
    source_tag_value TEXT NOT NULL,
    value_sort_index INTEGER NOT NULL DEFAULT 0,
    UNIQUE (discovery_title_id, source_tag_sort_index, source_tag_value)
);

CREATE TABLE IF NOT EXISTS discovery_title_external_ids (
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    source TEXT NOT NULL,
    external_kind TEXT NOT NULL DEFAULT '',
    external_id TEXT NOT NULL DEFAULT '',
    external_key TEXT NOT NULL DEFAULT '',
    external_identity TEXT NOT NULL DEFAULT '',
    sort_index INTEGER NOT NULL DEFAULT 0,
    UNIQUE (discovery_title_id, source, external_kind, external_identity)
);

CREATE TABLE IF NOT EXISTS discovery_items (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    base_generation_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    source_run_kind TEXT NOT NULL,
    section_id TEXT,
    sort_index INTEGER NOT NULL DEFAULT 0,
    best_source TEXT,
    source_count INTEGER,
    edge_count INTEGER,
    relation_count INTEGER,
    source_subject_count INTEGER,
    rank_score DOUBLE PRECISION,
    matched_subject_count INTEGER NOT NULL DEFAULT 0,
    owned_in_input BOOLEAN NOT NULL DEFAULT FALSE,
    tombstoned_by_run_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    tombstoned_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS discovery_section_items (
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    section_id TEXT NOT NULL,
    item_id TEXT NOT NULL REFERENCES discovery_items(id) ON DELETE CASCADE,
    sort_index INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (run_id, section_id, item_id)
);

CREATE TABLE IF NOT EXISTS discovery_item_rank_components (
    item_id TEXT NOT NULL REFERENCES discovery_items(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    component_index INTEGER NOT NULL,
    component_name TEXT NOT NULL DEFAULT '',
    component_value TEXT NOT NULL DEFAULT '',
    UNIQUE (item_id, component_index)
);

CREATE TABLE IF NOT EXISTS discovery_item_subject_links (
    item_id TEXT NOT NULL REFERENCES discovery_items(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    link_type TEXT NOT NULL,
    subject_key TEXT NOT NULL,
    sort_index INTEGER NOT NULL DEFAULT 0,
    UNIQUE (item_id, link_type, subject_key)
);

CREATE TABLE IF NOT EXISTS discovery_item_library_provenance (
    item_id TEXT NOT NULL REFERENCES discovery_items(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    subject_key TEXT NOT NULL,
    title_id TEXT NOT NULL DEFAULT '',
    library_id TEXT NOT NULL DEFAULT '',
    UNIQUE (item_id, subject_key, title_id, library_id)
);

CREATE TABLE IF NOT EXISTS discovery_facets (
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    facet_name TEXT NOT NULL,
    facet_value TEXT NOT NULL,
    smg_count BIGINT,
    local_count BIGINT,
    PRIMARY KEY (run_id, facet_name, facet_value)
);

CREATE INDEX IF NOT EXISTS idx_discovery_sync_runs_kind_status
    ON discovery_sync_runs(kind, status, updated_at);
CREATE INDEX IF NOT EXISTS idx_discovery_raw_pages_run
    ON discovery_raw_pages(run_id, payload_kind, page_number);
CREATE INDEX IF NOT EXISTS idx_discovery_submitted_subjects_title
    ON discovery_submitted_subjects(title_id);
CREATE INDEX IF NOT EXISTS idx_discovery_pending_changes_scope_seen
    ON discovery_pending_context_changes(scope_key, last_seen_at);
CREATE INDEX IF NOT EXISTS idx_discovery_pending_changes_scope_sequence
    ON discovery_pending_context_changes(scope_key, last_seen_sequence);
CREATE INDEX IF NOT EXISTS idx_discovery_sections_run_surface
    ON discovery_sections(run_id, surface, sort_index);
CREATE INDEX IF NOT EXISTS idx_canonical_media_subjects_title
    ON canonical_media_subjects(title_id);
CREATE INDEX IF NOT EXISTS idx_canonical_media_subjects_key_language
    ON canonical_media_subjects(subject_key_norm, language);
CREATE INDEX IF NOT EXISTS idx_canonical_media_tags_category_name
    ON canonical_media_tags(category, name, subject_id);
CREATE INDEX IF NOT EXISTS idx_discovery_titles_key_language
    ON discovery_titles(target_key_norm, language);
CREATE INDEX IF NOT EXISTS idx_discovery_title_terms_kind_value
    ON discovery_title_terms(term_kind, term_value, discovery_title_id);
CREATE INDEX IF NOT EXISTS idx_discovery_title_terms_title
    ON discovery_title_terms(discovery_title_id, term_kind, sort_index);
CREATE INDEX IF NOT EXISTS idx_discovery_title_source_tags_title
    ON discovery_title_source_tags(discovery_title_id, sort_index);
CREATE INDEX IF NOT EXISTS idx_discovery_title_source_tag_values_title
    ON discovery_title_source_tag_values(discovery_title_id, source_tag_sort_index, value_sort_index);
CREATE INDEX IF NOT EXISTS idx_discovery_title_external_ids_title
    ON discovery_title_external_ids(discovery_title_id, sort_index);
CREATE INDEX IF NOT EXISTS idx_discovery_items_active_title
    ON discovery_items(base_generation_id, discovery_title_id, tombstoned_at);
CREATE INDEX IF NOT EXISTS idx_discovery_items_run
    ON discovery_items(run_id);
CREATE INDEX IF NOT EXISTS idx_discovery_items_section
    ON discovery_items(section_id, sort_index, rank_score);
CREATE INDEX IF NOT EXISTS idx_discovery_items_run_section
    ON discovery_items(run_id, section_id, sort_index);
CREATE INDEX IF NOT EXISTS idx_discovery_section_items_run_section
    ON discovery_section_items(run_id, section_id, sort_index);
CREATE INDEX IF NOT EXISTS idx_discovery_item_rank_components_item
    ON discovery_item_rank_components(item_id, component_index);
CREATE INDEX IF NOT EXISTS idx_discovery_item_subject_links_run_type_key
    ON discovery_item_subject_links(run_id, link_type, subject_key, item_id);
CREATE INDEX IF NOT EXISTS idx_discovery_item_subject_links_item
    ON discovery_item_subject_links(item_id, link_type, sort_index);
CREATE INDEX IF NOT EXISTS idx_discovery_item_library_provenance_library
    ON discovery_item_library_provenance(run_id, library_id, item_id);
CREATE INDEX IF NOT EXISTS idx_discovery_item_library_provenance_item
    ON discovery_item_library_provenance(item_id, subject_key, library_id, title_id);
CREATE TABLE IF NOT EXISTS indexer_search_learning (
    indexer_id TEXT NOT NULL,
    title_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    strategy_key TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    empty_successes INTEGER NOT NULL DEFAULT 0,
    usable_successes INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TIMESTAMPTZ,
    last_usable_at TIMESTAMPTZ,
    suppressed BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (indexer_id, title_id, facet, strategy_key)
);

CREATE INDEX IF NOT EXISTS idx_indexer_search_learning_title
    ON indexer_search_learning (indexer_id, title_id, facet);
CREATE TABLE IF NOT EXISTS external_import_setup_secret_drafts (
    draft_key text PRIMARY KEY NOT NULL
        CHECK (draft_key = 'active'),
    owner_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL
);

CREATE TABLE IF NOT EXISTS external_import_setup_instance_api_keys (
    draft_key text NOT NULL REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE,
    instance_id text NOT NULL,
    kind text NOT NULL
        CHECK (kind IN ('sonarr', 'radarr', 'prowlarr')),
    api_key_encrypted text NOT NULL,
    position integer NOT NULL
        CHECK (position >= 0),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (draft_key, instance_id),
    UNIQUE (draft_key, position)
);

CREATE TABLE IF NOT EXISTS external_import_setup_download_client_api_key_overrides (
    draft_key text NOT NULL REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE,
    dedup_key text NOT NULL,
    api_key_encrypted text NOT NULL,
    position integer NOT NULL
        CHECK (position >= 0),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (draft_key, dedup_key),
    UNIQUE (draft_key, position)
);

CREATE TABLE IF NOT EXISTS external_import_setup_download_client_password_overrides (
    draft_key text NOT NULL REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE,
    dedup_key text NOT NULL,
    password_encrypted text NOT NULL,
    position integer NOT NULL
        CHECK (position >= 0),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (draft_key, dedup_key),
    UNIQUE (draft_key, position)
);

CREATE TABLE IF NOT EXISTS external_import_setup_indexer_api_key_overrides (
    draft_key text NOT NULL REFERENCES external_import_setup_secret_drafts(draft_key) ON DELETE CASCADE,
    dedup_key text NOT NULL,
    api_key_encrypted text NOT NULL,
    position integer NOT NULL
        CHECK (position >= 0),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    PRIMARY KEY (draft_key, dedup_key),
    UNIQUE (draft_key, position)
);
CREATE TABLE IF NOT EXISTS title_more_like_this_items (
    source_title_id text NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    discovery_title_id text NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    sort_index integer NOT NULL DEFAULT 0,
    rank_score double precision,
    best_source text,
    source_count integer,
    edge_count integer,
    relation_count integer,
    source_subject_count integer,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW(),
    UNIQUE (source_title_id, discovery_title_id)
);

CREATE INDEX IF NOT EXISTS idx_title_more_like_this_items_source_order
    ON title_more_like_this_items(source_title_id, sort_index ASC, rank_score DESC);

CREATE INDEX IF NOT EXISTS idx_title_more_like_this_items_title
    ON title_more_like_this_items(discovery_title_id);
CREATE TABLE IF NOT EXISTS upstream_scheduler_states (
    host_key TEXT NOT NULL,
    destination_key TEXT NOT NULL,
    account_quota_key TEXT NOT NULL DEFAULT '',
    rss_request_key TEXT NOT NULL DEFAULT '',
    api_current BIGINT,
    api_max BIGINT,
    grab_current BIGINT,
    grab_max BIGINT,
    quota_observed_at TIMESTAMPTZ,
    quota_probe_after TIMESTAMPTZ,
    quota_reset_at TIMESTAMPTZ,
    quota_source TEXT,
    last_decision TEXT,
    last_feedback_at TIMESTAMPTZ,
    last_successful_at TIMESTAMPTZ,
    last_attempt_at TIMESTAMPTZ,
    admitted_count BIGINT NOT NULL DEFAULT 0,
    deferred_count BIGINT NOT NULL DEFAULT 0,
    skipped_count BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (host_key, destination_key, account_quota_key, rss_request_key)
);

CREATE INDEX IF NOT EXISTS idx_upstream_scheduler_states_destination
    ON upstream_scheduler_states (destination_key);

CREATE TABLE IF NOT EXISTS upstream_destination_cooldowns (
    destination_key TEXT PRIMARY KEY,
    cooldown_until TIMESTAMPTZ NOT NULL,
    retry_after_seconds BIGINT,
    source TEXT NOT NULL,
    status_code BIGINT,
    message TEXT,
    observed_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_upstream_destination_cooldowns_until
    ON upstream_destination_cooldowns (cooldown_until);

CREATE TABLE IF NOT EXISTS upstream_scheduler_rss_cadence (
    host_key TEXT NOT NULL,
    destination_key TEXT NOT NULL,
    account_quota_key TEXT NOT NULL,
    rss_request_key TEXT NOT NULL DEFAULT '',
    last_successful_poll_at TIMESTAMPTZ,
    last_attempt_at TIMESTAMPTZ,
    target_interval_seconds BIGINT NOT NULL,
    latest_safe_poll_at TIMESTAMPTZ,
    estimated_feed_depth BIGINT,
    freshness_risk DOUBLE PRECISION NOT NULL DEFAULT 0,
    destination_recent_activity_at TIMESTAMPTZ,
    last_seen_release_identity TEXT,
    last_seen_release_published_at TIMESTAMPTZ,
    last_feed_gap_start_at TIMESTAMPTZ,
    last_feed_gap_end_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (host_key, destination_key, account_quota_key, rss_request_key)
);

CREATE INDEX IF NOT EXISTS idx_upstream_scheduler_rss_latest_safe_poll
    ON upstream_scheduler_rss_cadence (latest_safe_poll_at);

CREATE TABLE IF NOT EXISTS external_import_monitor_snapshot_chunks (
    session_id text NOT NULL,
    facet text NOT NULL
        CHECK (facet IN ('movie', 'series', 'anime')),
    entry_kind text NOT NULL
        CHECK (entry_kind IN ('movie', 'series')),
    chunk_index integer NOT NULL,
    payload_ndjson text NOT NULL,
    created_at timestamptz NOT NULL,
    PRIMARY KEY (session_id, facet, entry_kind, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_titles_catalog_sort_key
    ON titles(catalog_sort_key, name, year, id);

CREATE INDEX IF NOT EXISTS idx_titles_popularity
    ON titles(popularity);

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

CREATE TABLE IF NOT EXISTS canonical_media_rating_summaries (
    subject_id text PRIMARY KEY NOT NULL REFERENCES canonical_media_subjects(id) ON DELETE CASCADE,
    rating double precision,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS canonical_media_rating_sources (
    subject_id text NOT NULL REFERENCES canonical_media_subjects(id) ON DELETE CASCADE,
    source text NOT NULL,
    sort_index integer NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW(),
    PRIMARY KEY (subject_id, source)
);

CREATE TABLE IF NOT EXISTS canonical_media_external_ratings (
    subject_id text NOT NULL REFERENCES canonical_media_subjects(id) ON DELETE CASCADE,
    source text NOT NULL,
    sort_index integer NOT NULL DEFAULT 0,
    value double precision,
    score double precision,
    normalized double precision NOT NULL,
    votes integer,
    url text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT NOW(),
    updated_at timestamptz NOT NULL DEFAULT NOW(),
    PRIMARY KEY (subject_id, source)
);

CREATE INDEX IF NOT EXISTS idx_canonical_media_rating_sources_order
    ON canonical_media_rating_sources(subject_id, sort_index ASC, source ASC);

CREATE INDEX IF NOT EXISTS idx_canonical_media_external_ratings_order
    ON canonical_media_external_ratings(subject_id, sort_index ASC, source ASC);

CREATE INDEX IF NOT EXISTS idx_canonical_media_external_ratings_source_norm
    ON canonical_media_external_ratings(source, normalized, subject_id);

CREATE TABLE IF NOT EXISTS user_ui_settings (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    theme TEXT NOT NULL DEFAULT 'dark',
    date_time_format TEXT NOT NULL DEFAULT 'locale',
    highlight_color TEXT,
    secondary_color TEXT,
    high_contrast_mode BOOLEAN NOT NULL DEFAULT FALSE,
    reduce_motion BOOLEAN NOT NULL DEFAULT FALSE,
    hide_sponsor_button BOOLEAN NOT NULL DEFAULT FALSE,
    density TEXT NOT NULL DEFAULT 'comfortable',
    sidebar_mode TEXT NOT NULL DEFAULT 'expanded',
    default_landing_view TEXT NOT NULL DEFAULT 'movies',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS user_ui_table_columns (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    facet TEXT NOT NULL,
    table_view_mode TEXT NOT NULL,
    column_id TEXT NOT NULL,
    column_order INTEGER NOT NULL,
    visible BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, facet, table_view_mode, column_id)
);

CREATE INDEX IF NOT EXISTS idx_user_ui_table_columns_user_view
    ON user_ui_table_columns(user_id, facet, table_view_mode, column_order);
