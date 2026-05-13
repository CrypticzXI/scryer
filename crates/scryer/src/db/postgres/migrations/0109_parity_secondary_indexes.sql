CREATE INDEX IF NOT EXISTS idx_blocklist_source_title
    ON blocklist (source_title)
    WHERE source_title IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_blocklist_title_id
    ON blocklist (title_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_collection_external_ids_unique
    ON collection_external_ids(collection_id, source, external_id, provenance, source_scope);

CREATE INDEX IF NOT EXISTS idx_collections_title
    ON collections (title_id, collection_type);

CREATE INDEX IF NOT EXISTS idx_domain_events_event_type_sequence
    ON domain_events (event_type, sequence DESC);

CREATE INDEX IF NOT EXISTS idx_domain_events_facet_sequence
    ON domain_events (facet, sequence DESC);

CREATE INDEX IF NOT EXISTS idx_domain_events_occurred_at
    ON domain_events (occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_domain_events_stream_sequence
    ON domain_events (stream_kind, stream_id, sequence DESC);

CREATE INDEX IF NOT EXISTS idx_domain_events_title_sequence
    ON domain_events (title_id, sequence DESC);

CREATE INDEX IF NOT EXISTS idx_download_clients_client_priority
    ON download_clients (client_priority);

CREATE UNIQUE INDEX IF NOT EXISTS idx_download_clients_name
    ON download_clients (name);

CREATE INDEX IF NOT EXISTS idx_download_import_artifacts_episode
    ON download_import_artifacts (episode_id, result);

CREATE INDEX IF NOT EXISTS idx_download_import_artifacts_retention
    ON download_import_artifacts (created_at, import_id);

CREATE INDEX IF NOT EXISTS idx_download_import_artifacts_source
    ON download_import_artifacts (source_system, source_ref, created_at);

CREATE INDEX IF NOT EXISTS idx_download_queue_commands_source
    ON download_queue_commands (
        COALESCE(client_id, ''),
        client_type,
        download_client_item_id,
        is_history,
        created_at DESC
    );

CREATE INDEX IF NOT EXISTS idx_download_queue_commands_status
    ON download_queue_commands (action, status, updated_at);

CREATE INDEX IF NOT EXISTS idx_download_submission_episode_links_episode
    ON download_submission_episode_links (episode_id);

CREATE INDEX IF NOT EXISTS idx_download_submissions_title_request_signature
    ON download_submissions (title_id, request_signature);

CREATE UNIQUE INDEX IF NOT EXISTS idx_episode_external_ids_unique
    ON episode_external_ids(episode_id, source, external_id, provenance, source_scope);

CREATE INDEX IF NOT EXISTS idx_episodes_collection
    ON episodes (collection_id);

CREATE INDEX IF NOT EXISTS idx_episodes_title
    ON episodes (title_id, season_number);

CREATE INDEX IF NOT EXISTS idx_event_outboxes_status
    ON event_outboxes (status, updated_at);

CREATE INDEX IF NOT EXISTS idx_external_subtitle_probe_cache_file_path
    ON external_subtitle_probe_cache (file_path);

CREATE INDEX IF NOT EXISTS idx_external_subtitle_probe_cache_media_file
    ON external_subtitle_probe_cache (media_file_id);

CREATE INDEX IF NOT EXISTS idx_file_episode_map_episode
    ON file_episode_map (episode_id);

CREATE INDEX IF NOT EXISTS idx_history_events_occurred_at
    ON history_events (occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_history_events_title_time
    ON history_events (title_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS idx_history_events_type_time
    ON history_events (event_type, occurred_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_imports_source_ref
    ON imports (source_system, source_ref, import_type);

CREATE INDEX IF NOT EXISTS idx_imports_status_updated_at
    ON imports (status, updated_at);

CREATE INDEX IF NOT EXISTS idx_library_probe_signatures_last_probed
    ON library_probe_signatures (last_probed_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_library_scan_unmatched_items_facet_path
    ON library_scan_unmatched_items (facet, item_path);

CREATE INDEX IF NOT EXISTS idx_library_scan_unmatched_items_facet_status_updated
    ON library_scan_unmatched_items (facet, status, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_library_scan_unmatched_items_facet_updated
    ON library_scan_unmatched_items (facet, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_library_scan_unmatched_items_root_status_updated
    ON library_scan_unmatched_items (facet, scan_root, status, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_library_scan_unmatched_items_root_updated
    ON library_scan_unmatched_items (facet, scan_root, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_media_files_title
    ON media_files (title_id);

CREATE INDEX IF NOT EXISTS idx_media_files_title_path
    ON media_files (title_id, file_path);

CREATE UNIQUE INDEX IF NOT EXISTS idx_notification_channels_name_type
    ON notification_channels (name, channel_type);

CREATE UNIQUE INDEX IF NOT EXISTS idx_notification_subscriptions_channel_scope
    ON notification_subscriptions (
        channel_id,
        event_type,
        COALESCE(scope, ''),
        COALESCE(scope_id, '')
    );

CREATE INDEX IF NOT EXISTS idx_operations_status_time
    ON workflow_operations (status, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_pending_releases_status
    ON pending_releases (status);

CREATE INDEX IF NOT EXISTS idx_pending_releases_wanted
    ON pending_releases (wanted_item_id, status);

CREATE INDEX IF NOT EXISTS idx_plugin_catalog_sources_kind
    ON plugin_catalog_sources (source_kind);

CREATE INDEX IF NOT EXISTS idx_pp_script_runs_script_id
    ON post_processing_script_runs (script_id, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_push_subscriptions_user_id
    ON push_subscriptions (user_id);

CREATE INDEX IF NOT EXISTS idx_quality_profile_audio_codec_allowlist_profile
    ON quality_profile_audio_codec_allowlist (profile_id);

CREATE INDEX IF NOT EXISTS idx_quality_profile_audio_codec_blocklist_profile
    ON quality_profile_audio_codec_blocklist (profile_id);

CREATE INDEX IF NOT EXISTS idx_quality_profile_source_allowlist_profile
    ON quality_profile_source_allowlist (profile_id);

CREATE INDEX IF NOT EXISTS idx_quality_profile_source_blocklist_profile
    ON quality_profile_source_blocklist (profile_id);

CREATE INDEX IF NOT EXISTS idx_quality_profile_video_codec_allowlist_profile
    ON quality_profile_video_codec_allowlist (profile_id);

CREATE INDEX IF NOT EXISTS idx_quality_profile_video_codec_blocklist_profile
    ON quality_profile_video_codec_blocklist (profile_id);

CREATE INDEX IF NOT EXISTS idx_quality_profiles_scope
    ON quality_profiles (scope, scope_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_quarantine_items_file
    ON quarantine_items (file_path);

CREATE INDEX IF NOT EXISTS idx_release_decisions_created_at
    ON release_decisions (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_release_decisions_wanted
    ON release_decisions (wanted_item_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_release_download_attempts_outcome_attempted
    ON release_download_attempts (outcome, attempted_at DESC);

CREATE INDEX IF NOT EXISTS idx_release_download_attempts_source_hint
    ON release_download_attempts (source_hint);

CREATE INDEX IF NOT EXISTS idx_release_download_attempts_source_title
    ON release_download_attempts (source_title);

CREATE INDEX IF NOT EXISTS idx_releases_collection
    ON releases (collection_id);

CREATE INDEX IF NOT EXISTS idx_releases_indexer_id
    ON releases (indexer_id);

CREATE INDEX IF NOT EXISTS idx_releases_title
    ON releases (title_id);

CREATE INDEX IF NOT EXISTS idx_releases_title_scope
    ON releases (title_id, release_scope);

CREATE INDEX IF NOT EXISTS idx_rule_set_history_created_at
    ON rule_set_history (created_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_rule_sets_managed_key
    ON rule_sets (managed_key)
    WHERE managed_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_scheduler_jobs_name
    ON scheduler_jobs (job_name);

CREATE INDEX IF NOT EXISTS idx_scheduler_jobs_status_next_run
    ON scheduler_jobs (status, next_run_at);

CREATE INDEX IF NOT EXISTS idx_subtitle_blocklist_media_file
    ON subtitle_blocklist (media_file_id);

CREATE INDEX IF NOT EXISTS idx_subtitle_downloads_language
    ON subtitle_downloads (language);

CREATE INDEX IF NOT EXISTS idx_subtitle_downloads_media_file
    ON subtitle_downloads (media_file_id);

CREATE INDEX IF NOT EXISTS idx_subtitle_downloads_title
    ON subtitle_downloads (title_id);

CREATE INDEX IF NOT EXISTS idx_subtitle_provider_configs_disabled_until
    ON subtitle_provider_configs (disabled_until);

CREATE INDEX IF NOT EXISTS idx_subtitle_provider_configs_enabled
    ON subtitle_provider_configs (is_enabled);

CREATE INDEX IF NOT EXISTS idx_subtitle_provider_configs_provider_type
    ON subtitle_provider_configs (provider_type);

CREATE UNIQUE INDEX IF NOT EXISTS idx_title_aliases_title_alias
    ON title_aliases (title_id, alias_type, alias_value);

CREATE UNIQUE INDEX IF NOT EXISTS idx_title_external_ids_facet_lookup
    ON title_external_ids (facet, source, external_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_title_external_ids_lookup
    ON title_external_ids (title_id, source, external_id);

CREATE INDEX IF NOT EXISTS idx_title_external_ids_title_id
    ON title_external_ids (title_id);

CREATE INDEX IF NOT EXISTS idx_title_search_terms_normalized_term
    ON title_search_terms (normalized_term);

CREATE INDEX IF NOT EXISTS idx_titles_facet_monitored
    ON titles (facet, monitored);

CREATE INDEX IF NOT EXISTS idx_titles_facet_normalized_slug
    ON titles (facet, LOWER(TRIM(slug)))
    WHERE slug IS NOT NULL AND TRIM(slug) <> '';

CREATE INDEX IF NOT EXISTS idx_upgrades_status
    ON upgrades (status);

CREATE INDEX IF NOT EXISTS idx_user_entitlements_user
    ON user_entitlements (user_id);

CREATE INDEX IF NOT EXISTS idx_wanted_items_next_search
    ON wanted_items (status, next_search_at);

CREATE INDEX IF NOT EXISTS idx_wanted_items_title
    ON wanted_items (title_id);

CREATE INDEX IF NOT EXISTS idx_workflow_operations_job_key_started
    ON workflow_operations (job_key, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_workflow_operations_job_key_status
    ON workflow_operations (job_key, status, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_workflow_operations_status_started
    ON workflow_operations (status, started_at);
