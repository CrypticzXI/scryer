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
    sort_index INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS discovery_titles (
    id TEXT PRIMARY KEY NOT NULL,
    target_key TEXT NOT NULL,
    target_key_norm TEXT NOT NULL,
    language TEXT NOT NULL,
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
    rating REAL,
    tmdb_collection_id TEXT,
    tmdb_collection_name TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
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
    sort_index INTEGER NOT NULL DEFAULT 0,
    UNIQUE (discovery_title_id, source, external_kind, external_id, external_key)
);

CREATE TABLE IF NOT EXISTS discovery_title_ratings (
    discovery_title_id TEXT NOT NULL REFERENCES discovery_titles(id) ON DELETE CASCADE,
    rating_source TEXT NOT NULL,
    rating_value REAL,
    rating_score REAL,
    normalized REAL,
    votes INTEGER,
    url TEXT NOT NULL DEFAULT '',
    sort_index INTEGER NOT NULL DEFAULT 0,
    UNIQUE (discovery_title_id, rating_source)
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
    rank_score REAL,
    matched_subject_count INTEGER NOT NULL DEFAULT 0,
    owned_in_input INTEGER NOT NULL DEFAULT 0,
    tombstoned_by_run_id TEXT REFERENCES discovery_sync_runs(id) ON DELETE SET NULL,
    tombstoned_at TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
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
    smg_count INTEGER,
    local_count INTEGER,
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
CREATE INDEX IF NOT EXISTS idx_discovery_title_ratings_title
    ON discovery_title_ratings(discovery_title_id, sort_index);
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
