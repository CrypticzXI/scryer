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
    library_facet TEXT,
    title_kind TEXT,
    display_title TEXT,
    external_ids_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    raw_subject_json JSONB NOT NULL,
    PRIMARY KEY (run_id, subject_key)
);

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
    source_signals_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    facets_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    sort_index INTEGER NOT NULL DEFAULT 0,
    raw_json JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS discovery_items (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    base_generation_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    source_run_kind TEXT NOT NULL,
    section_id TEXT,
    target_key TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    resolved BOOLEAN NOT NULL DEFAULT FALSE,
    resolved_title_id TEXT REFERENCES titles(id) ON DELETE SET NULL,
    display_title TEXT NOT NULL,
    original_title TEXT,
    sort_title TEXT,
    year INTEGER,
    poster_path TEXT,
    poster_url TEXT,
    background_url TEXT,
    overview TEXT,
    content_type TEXT,
    genres_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    rating DOUBLE PRECISION,
    rating_sources_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    status_tags_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    source_tags_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    sources_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    best_source TEXT,
    relation_types_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    relation_subtypes_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    chart_signals_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    provider_signals_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    rank_components_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    source_count INTEGER,
    edge_count INTEGER,
    relation_count INTEGER,
    source_subject_count INTEGER,
    rank_score DOUBLE PRECISION,
    matched_subject_keys_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    matched_subject_titles_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    matched_subject_count INTEGER NOT NULL DEFAULT 0,
    tmdb_collection_id TEXT,
    tmdb_collection_name TEXT,
    owned_in_input BOOLEAN NOT NULL DEFAULT FALSE,
    facet_terms_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    context_terms_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    change_subject_keys_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    removed_subject_keys_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    tombstoned_by_run_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    tombstoned_at TIMESTAMPTZ,
    raw_json JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS discovery_facets (
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    facet_name TEXT NOT NULL,
    facet_value TEXT NOT NULL,
    smg_count BIGINT,
    local_count BIGINT,
    raw_json JSONB NOT NULL,
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
CREATE INDEX IF NOT EXISTS idx_discovery_items_active_target
    ON discovery_items(base_generation_id, target_key, tombstoned_at);
CREATE INDEX IF NOT EXISTS idx_discovery_items_run
    ON discovery_items(run_id);
CREATE INDEX IF NOT EXISTS idx_discovery_items_section
    ON discovery_items(section_id, rank_score);
CREATE INDEX IF NOT EXISTS idx_discovery_items_target_kind
    ON discovery_items(target_kind, resolved, owned_in_input);
