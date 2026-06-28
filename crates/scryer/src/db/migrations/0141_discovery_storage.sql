CREATE TABLE IF NOT EXISTS discovery_sync_runs (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    trigger_source TEXT NOT NULL,
    region TEXT NOT NULL,
    language TEXT NOT NULL,
    subject_count INTEGER NOT NULL DEFAULT 0,
    subject_fingerprint TEXT,
    previous_subject_fingerprint TEXT,
    base_generation_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    changed_subject_count INTEGER NOT NULL DEFAULT 0,
    affected_target_count INTEGER NOT NULL DEFAULT 0,
    smg_request_id TEXT,
    smg_status TEXT,
    discovery_index_watermark TEXT,
    page_count INTEGER,
    item_count INTEGER,
    facet_count INTEGER,
    raw_submit_json TEXT,
    raw_changes_json TEXT,
    raw_final_status_json TEXT,
    raw_ack_json TEXT,
    error_text TEXT,
    started_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS discovery_sync_state (
    scope_key TEXT PRIMARY KEY NOT NULL,
    last_success_generation_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    last_public_feed_generation_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    last_subject_fingerprint TEXT,
    last_context_snapshot_completed_at TEXT,
    last_incremental_reload_completed_at TEXT,
    last_public_feed_completed_at TEXT,
    dirty_since TEXT,
    dirty_reason_mask INTEGER NOT NULL DEFAULT 0,
    bootstrap_started_at TEXT,
    bootstrap_quiet_until TEXT,
    next_context_snapshot_eligible_at TEXT,
    next_incremental_reload_eligible_at TEXT,
    next_public_feed_eligible_at TEXT,
    backoff_until TEXT,
    startup_jitter_seconds INTEGER NOT NULL DEFAULT 0,
    context_jitter_seconds INTEGER NOT NULL DEFAULT 0,
    incremental_reload_jitter_seconds INTEGER NOT NULL DEFAULT 0,
    public_feed_jitter_seconds INTEGER NOT NULL DEFAULT 0,
    last_seen_domain_event_sequence INTEGER,
    inflight_subject_fingerprint TEXT,
    inflight_domain_event_sequence INTEGER,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS discovery_raw_pages (
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    payload_kind TEXT NOT NULL,
    page_number INTEGER NOT NULL DEFAULT 0,
    compression TEXT NOT NULL DEFAULT 'none',
    raw_payload TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
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
    external_ids_json TEXT NOT NULL DEFAULT '[]',
    raw_subject_json TEXT NOT NULL
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
    raw_subject_json TEXT,
    raw_previous_subject_json TEXT,
    first_seen_sequence INTEGER,
    last_seen_sequence INTEGER,
    first_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS discovery_sections (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    section_id TEXT NOT NULL,
    section_type TEXT NOT NULL,
    surface TEXT NOT NULL,
    title TEXT NOT NULL,
    source_signals_json TEXT NOT NULL DEFAULT '[]',
    facets_json TEXT NOT NULL DEFAULT '[]',
    sort_index INTEGER NOT NULL DEFAULT 0,
    raw_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS discovery_items (
    id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    base_generation_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    source_run_kind TEXT NOT NULL,
    section_id TEXT,
    target_key TEXT NOT NULL,
    target_kind TEXT NOT NULL,
    resolved INTEGER NOT NULL DEFAULT 0,
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
    genres_json TEXT NOT NULL DEFAULT '[]',
    rating REAL,
    rating_sources_json TEXT NOT NULL DEFAULT '[]',
    status_tags_json TEXT NOT NULL DEFAULT '[]',
    source_tags_json TEXT NOT NULL DEFAULT '[]',
    sources_json TEXT NOT NULL DEFAULT '[]',
    best_source TEXT,
    relation_types_json TEXT NOT NULL DEFAULT '[]',
    relation_subtypes_json TEXT NOT NULL DEFAULT '[]',
    chart_signals_json TEXT NOT NULL DEFAULT '[]',
    provider_signals_json TEXT NOT NULL DEFAULT '[]',
    rank_components_json TEXT NOT NULL DEFAULT '[]',
    source_count INTEGER,
    edge_count INTEGER,
    relation_count INTEGER,
    source_subject_count INTEGER,
    rank_score REAL,
    matched_subject_keys_json TEXT NOT NULL DEFAULT '[]',
    matched_subject_titles_json TEXT NOT NULL DEFAULT '[]',
    matched_subject_count INTEGER NOT NULL DEFAULT 0,
    library_provenance_json TEXT NOT NULL DEFAULT '[]',
    tmdb_collection_id TEXT,
    tmdb_collection_name TEXT,
    owned_in_input INTEGER NOT NULL DEFAULT 0,
    facet_terms_json TEXT NOT NULL DEFAULT '[]',
    context_terms_json TEXT NOT NULL DEFAULT '[]',
    change_subject_keys_json TEXT NOT NULL DEFAULT '[]',
    removed_subject_keys_json TEXT NOT NULL DEFAULT '[]',
    tombstoned_by_run_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    tombstoned_at TEXT,
    raw_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS discovery_facets (
    run_id TEXT NOT NULL REFERENCES discovery_sync_runs(id) ON DELETE CASCADE,
    facet_name TEXT NOT NULL,
    facet_value TEXT NOT NULL,
    smg_count INTEGER,
    local_count INTEGER,
    raw_json TEXT NOT NULL,
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
