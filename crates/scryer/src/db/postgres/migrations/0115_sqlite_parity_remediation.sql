-- PostgreSQL remediation to match the canonical SQLite 0114 logical model.

ALTER TABLE workflow_operations
    ADD COLUMN IF NOT EXISTS title_id text,
    ADD COLUMN IF NOT EXISTS collection_id text,
    ADD COLUMN IF NOT EXISTS episode_id text,
    ADD COLUMN IF NOT EXISTS release_id text,
    ADD COLUMN IF NOT EXISTS media_file_id text,
    ADD COLUMN IF NOT EXISTS external_reference text;
ALTER TABLE workflow_operations ALTER COLUMN status SET DEFAULT 'queued';

INSERT INTO workflow_operations (
    id,
    operation_type,
    status,
    job_key,
    trigger_source,
    actor_user_id,
    progress_json,
    summary_json,
    summary_text,
    error_text,
    started_at,
    completed_at,
    created_at,
    updated_at
)
SELECT
    id,
    job_key,
    status,
    job_key,
    'system_internal',
    NULL,
    progress_json,
    summary_json,
    NULL,
    error_text,
    started_at,
    completed_at,
    created_at,
    updated_at
FROM download_jobs
ON CONFLICT (id) DO UPDATE SET
    operation_type = EXCLUDED.operation_type,
    status = EXCLUDED.status,
    job_key = EXCLUDED.job_key,
    trigger_source = EXCLUDED.trigger_source,
    progress_json = EXCLUDED.progress_json,
    summary_json = EXCLUDED.summary_json,
    error_text = EXCLUDED.error_text,
    started_at = EXCLUDED.started_at,
    completed_at = EXCLUDED.completed_at,
    updated_at = EXCLUDED.updated_at;

INSERT INTO workflow_operations (
    id,
    operation_type,
    status,
    job_key,
    trigger_source,
    actor_user_id,
    progress_json,
    summary_json,
    summary_text,
    error_text,
    started_at,
    completed_at,
    created_at,
    updated_at
)
SELECT
    id,
    job_key,
    status,
    job_key,
    'system_internal',
    NULL,
    progress_json,
    summary_json,
    NULL,
    error_text,
    started_at,
    completed_at,
    created_at,
    updated_at
FROM job_runs
ON CONFLICT (id) DO UPDATE SET
    operation_type = EXCLUDED.operation_type,
    status = EXCLUDED.status,
    job_key = EXCLUDED.job_key,
    trigger_source = EXCLUDED.trigger_source,
    progress_json = EXCLUDED.progress_json,
    summary_json = EXCLUDED.summary_json,
    error_text = EXCLUDED.error_text,
    started_at = EXCLUDED.started_at,
    completed_at = EXCLUDED.completed_at,
    updated_at = EXCLUDED.updated_at;

DROP TABLE download_jobs;
CREATE TABLE download_jobs (
    id text NOT NULL,
    workflow_operation_id text NOT NULL,
    download_client_id text NOT NULL,
    release_id text,
    source_hint text,
    payload_json jsonb,
    status text DEFAULT 'pending'::text NOT NULL,
    attempts bigint DEFAULT 0 NOT NULL,
    last_error text,
    started_at timestamp with time zone,
    completed_at timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);
ALTER TABLE ONLY download_jobs
    ADD CONSTRAINT download_jobs_pkey PRIMARY KEY (id);

ALTER TABLE download_submission_episode_links
    ADD COLUMN download_client_id text,
    ADD COLUMN download_client_type text,
    ADD COLUMN download_client_item_id text;

UPDATE download_submission_episode_links links
SET
    download_client_id = submissions.download_client_id,
    download_client_type = submissions.download_client_type,
    download_client_item_id = submissions.download_client_item_id
FROM download_submissions submissions
WHERE submissions.id = links.submission_id;

DELETE FROM download_submission_episode_links
WHERE download_client_type IS NULL
   OR download_client_item_id IS NULL;

ALTER TABLE download_submission_episode_links
    DROP CONSTRAINT download_submission_episode_links_pkey,
    DROP COLUMN submission_id,
    DROP COLUMN created_at;
UPDATE download_submission_episode_links
SET download_client_id = COALESCE(download_client_id, '');
ALTER TABLE download_submission_episode_links
    ALTER COLUMN download_client_id SET DEFAULT '',
    ALTER COLUMN download_client_id SET NOT NULL,
    ALTER COLUMN download_client_type SET NOT NULL,
    ALTER COLUMN download_client_item_id SET NOT NULL,
    ALTER COLUMN episode_id SET NOT NULL;
ALTER TABLE ONLY download_submission_episode_links
    ADD CONSTRAINT download_submission_episode_links_pkey
    PRIMARY KEY (download_client_id, download_client_type, download_client_item_id, episode_id);
INSERT INTO download_submission_episode_links (
    download_client_id,
    download_client_type,
    download_client_item_id,
    episode_id
)
SELECT
    COALESCE(download_client_id, ''),
    download_client_type,
    download_client_item_id,
    NULLIF(TRIM(episode_values.episode_id), '')
FROM download_submissions
CROSS JOIN LATERAL regexp_split_to_table(COALESCE(episode_set_ids, ''), CHR(31)) AS episode_values(episode_id)
WHERE NULLIF(TRIM(episode_values.episode_id), '') IS NOT NULL
ON CONFLICT DO NOTHING;
ALTER TABLE download_submissions DROP COLUMN episode_set_ids;

ALTER TABLE collection_external_ids ADD COLUMN title_id text;
UPDATE collection_external_ids external_ids
SET title_id = collections.title_id
FROM collections
WHERE collections.id = external_ids.collection_id;
DELETE FROM collection_external_ids WHERE title_id IS NULL OR collection_id IS NULL;
UPDATE collection_external_ids
SET
    source = COALESCE(NULLIF(source, ''), 'unknown'),
    external_id = COALESCE(NULLIF(external_id, ''), id),
    provenance = COALESCE(NULLIF(provenance, ''), 'metadata'),
    source_scope = COALESCE(source_scope, ''),
    created_at = COALESCE(created_at, NOW());
ALTER TABLE collection_external_ids
    ALTER COLUMN title_id SET NOT NULL,
    ALTER COLUMN collection_id SET NOT NULL,
    ALTER COLUMN source SET NOT NULL,
    ALTER COLUMN external_id SET NOT NULL,
    ALTER COLUMN provenance SET DEFAULT 'metadata',
    ALTER COLUMN provenance SET NOT NULL,
    ALTER COLUMN source_scope SET DEFAULT '',
    ALTER COLUMN source_scope SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL;

ALTER TABLE episode_external_ids ADD COLUMN title_id text;
UPDATE episode_external_ids external_ids
SET title_id = episodes.title_id
FROM episodes
WHERE episodes.id = external_ids.episode_id;
DELETE FROM episode_external_ids WHERE title_id IS NULL OR episode_id IS NULL;
UPDATE episode_external_ids
SET
    source = COALESCE(NULLIF(source, ''), 'unknown'),
    external_id = COALESCE(NULLIF(external_id, ''), id),
    provenance = COALESCE(NULLIF(provenance, ''), 'metadata'),
    source_scope = COALESCE(source_scope, ''),
    created_at = COALESCE(created_at, NOW());
ALTER TABLE episode_external_ids
    ALTER COLUMN title_id SET NOT NULL,
    ALTER COLUMN episode_id SET NOT NULL,
    ALTER COLUMN source SET NOT NULL,
    ALTER COLUMN external_id SET NOT NULL,
    ALTER COLUMN provenance SET DEFAULT 'metadata',
    ALTER COLUMN provenance SET NOT NULL,
    ALTER COLUMN source_scope SET DEFAULT '',
    ALTER COLUMN source_scope SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL;

ALTER TABLE collections
    ADD COLUMN interstitial_tvdb_id text,
    ADD COLUMN interstitial_name text,
    ADD COLUMN interstitial_slug text,
    ADD COLUMN interstitial_year integer,
    ADD COLUMN interstitial_content_status text,
    ADD COLUMN interstitial_overview text,
    ADD COLUMN interstitial_poster_url text,
    ADD COLUMN interstitial_language text,
    ADD COLUMN interstitial_runtime_minutes bigint,
    ADD COLUMN interstitial_sort_title text,
    ADD COLUMN interstitial_imdb_id text,
    ADD COLUMN interstitial_genres_json jsonb,
    ADD COLUMN interstitial_studio text,
    ADD COLUMN interstitial_digital_release_date text,
    ADD COLUMN interstitial_association_confidence text,
    ADD COLUMN interstitial_continuity_status text,
    ADD COLUMN interstitial_movie_form text,
    ADD COLUMN interstitial_confidence text,
    ADD COLUMN interstitial_signal_summary text,
    ADD COLUMN interstitial_placement text,
    ADD COLUMN interstitial_movie_tmdb_id text,
    ADD COLUMN interstitial_movie_mal_id text,
    ADD COLUMN interstitial_movie_anidb_id text,
    ADD COLUMN special_movies_json jsonb;
UPDATE collections
SET
    interstitial_tvdb_id = COALESCE(interstitial_tvdb_id, interstitial_movie_json ->> 'tvdb_id'),
    interstitial_name = COALESCE(interstitial_name, interstitial_movie_json ->> 'name'),
    interstitial_slug = COALESCE(interstitial_slug, interstitial_movie_json ->> 'slug'),
    interstitial_year = COALESCE(interstitial_year, NULLIF(interstitial_movie_json ->> 'year', '')::integer),
    interstitial_content_status = COALESCE(interstitial_content_status, interstitial_movie_json ->> 'content_status'),
    interstitial_overview = COALESCE(interstitial_overview, interstitial_movie_json ->> 'overview'),
    interstitial_poster_url = COALESCE(interstitial_poster_url, interstitial_movie_json ->> 'poster_url'),
    interstitial_language = COALESCE(interstitial_language, interstitial_movie_json ->> 'language'),
    interstitial_runtime_minutes = COALESCE(interstitial_runtime_minutes, NULLIF(interstitial_movie_json ->> 'runtime_minutes', '')::bigint),
    interstitial_sort_title = COALESCE(interstitial_sort_title, interstitial_movie_json ->> 'sort_title'),
    interstitial_imdb_id = COALESCE(interstitial_imdb_id, interstitial_movie_json ->> 'imdb_id'),
    interstitial_genres_json = COALESCE(interstitial_genres_json, interstitial_movie_json -> 'genres'),
    interstitial_studio = COALESCE(interstitial_studio, interstitial_movie_json ->> 'studio'),
    interstitial_digital_release_date = COALESCE(interstitial_digital_release_date, interstitial_movie_json ->> 'digital_release_date'),
    interstitial_association_confidence = COALESCE(interstitial_association_confidence, interstitial_movie_json ->> 'association_confidence'),
    interstitial_continuity_status = COALESCE(interstitial_continuity_status, interstitial_movie_json ->> 'continuity_status'),
    interstitial_movie_form = COALESCE(interstitial_movie_form, interstitial_movie_json ->> 'movie_form'),
    interstitial_confidence = COALESCE(interstitial_confidence, interstitial_movie_json ->> 'confidence'),
    interstitial_signal_summary = COALESCE(interstitial_signal_summary, interstitial_movie_json ->> 'signal_summary'),
    interstitial_placement = COALESCE(interstitial_placement, interstitial_movie_json ->> 'placement'),
    interstitial_movie_tmdb_id = COALESCE(interstitial_movie_tmdb_id, interstitial_movie_json ->> 'tmdb_id'),
    interstitial_movie_mal_id = COALESCE(interstitial_movie_mal_id, interstitial_movie_json ->> 'mal_id'),
    interstitial_movie_anidb_id = COALESCE(interstitial_movie_anidb_id, interstitial_movie_json ->> 'anidb_id'),
    monitored = COALESCE(monitored, true),
    special_movies_json = COALESCE(special_movies_json, specials_movies_json, '[]'::jsonb);
ALTER TABLE collections
    ALTER COLUMN monitored SET DEFAULT true,
    ALTER COLUMN monitored SET NOT NULL,
    ALTER COLUMN special_movies_json SET DEFAULT '[]'::jsonb,
    ALTER COLUMN special_movies_json SET NOT NULL,
    DROP COLUMN interstitial_movie_json,
    DROP COLUMN specials_movies_json;

ALTER TABLE history_events
    ADD COLUMN actor_user_id text,
    ADD COLUMN message text,
    ADD COLUMN metadata_json jsonb,
    ADD COLUMN source text;
UPDATE history_events
SET
    message = COALESCE(NULLIF(message, ''), event_json ->> 'message', event_type, id),
    metadata_json = COALESCE(metadata_json, event_json),
    source = COALESCE(source, event_json ->> 'source'),
    occurred_at = COALESCE(occurred_at, created_at, NOW()),
    created_at = COALESCE(created_at, NOW());
ALTER TABLE history_events
    ALTER COLUMN message SET NOT NULL,
    ALTER COLUMN occurred_at SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    DROP COLUMN event_json;

ALTER TABLE event_outboxes
    ADD COLUMN history_event_id text,
    ADD COLUMN channel_key text,
    ADD COLUMN attempt_count bigint DEFAULT 0 NOT NULL,
    ADD COLUMN last_error text;
UPDATE event_outboxes
SET
    history_event_id = COALESCE(event_id, id),
    channel_key = COALESCE(NULLIF(subscriber, ''), 'default'),
    payload_json = COALESCE(payload_json, '{}'::jsonb),
    status = COALESCE(NULLIF(status, ''), 'pending'),
    created_at = COALESCE(created_at, NOW()),
    updated_at = COALESCE(updated_at, NOW());
INSERT INTO history_events (id, event_type, message, occurred_at, created_at)
SELECT DISTINCT history_event_id, 'event_outbox', 'Event outbox entry', created_at, created_at
FROM event_outboxes
WHERE history_event_id IS NOT NULL
ON CONFLICT (id) DO NOTHING;
ALTER TABLE event_outboxes
    ALTER COLUMN history_event_id SET NOT NULL,
    ALTER COLUMN channel_key SET NOT NULL,
    ALTER COLUMN payload_json SET NOT NULL,
    ALTER COLUMN status SET DEFAULT 'pending',
    ALTER COLUMN status SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET NOT NULL,
    DROP COLUMN event_id,
    DROP COLUMN subscriber;

ALTER TABLE indexer_api_quotas RENAME TO indexer_api_quotas_old;
CREATE TABLE indexer_api_quotas (
    indexer_id text NOT NULL,
    api_current bigint,
    api_max bigint,
    grab_current bigint,
    grab_max bigint,
    queries_today bigint DEFAULT 0 NOT NULL,
    last_query_at timestamp with time zone,
    last_reset_at timestamp with time zone DEFAULT NOW() NOT NULL,
    updated_at timestamp with time zone DEFAULT NOW() NOT NULL
);
INSERT INTO indexer_api_quotas (
    indexer_id,
    queries_today,
    last_reset_at,
    updated_at
)
SELECT DISTINCT ON (COALESCE(indexer_id, id))
    COALESCE(indexer_id, id),
    COALESCE(used_count, 0),
    COALESCE(reset_at, NOW()),
    COALESCE(updated_at, NOW())
FROM indexer_api_quotas_old
WHERE COALESCE(indexer_id, id) IS NOT NULL
ORDER BY COALESCE(indexer_id, id), updated_at DESC NULLS LAST;
DROP TABLE indexer_api_quotas_old;
ALTER TABLE ONLY indexer_api_quotas
    ADD CONSTRAINT indexer_api_quotas_pkey PRIMARY KEY (indexer_id);

ALTER TABLE integration_tokens RENAME TO integration_tokens_old;
CREATE TABLE integration_tokens (
    id text NOT NULL,
    user_id text NOT NULL,
    token_name text,
    token_hash text NOT NULL,
    scopes_json jsonb NOT NULL,
    created_at timestamp with time zone NOT NULL,
    created_by_user_id text,
    expires_at timestamp with time zone,
    revoked_at timestamp with time zone,
    last_used_at timestamp with time zone
);
INSERT INTO integration_tokens (
    id,
    user_id,
    token_name,
    token_hash,
    scopes_json,
    created_at
)
SELECT
    old.id,
    fallback_user.id,
    old.token_name,
    COALESCE(old.token_secret, old.integration_key, old.id),
    COALESCE(old.metadata_json -> 'scopes', '[]'::jsonb),
    COALESCE(old.created_at, NOW())
FROM integration_tokens_old old
CROSS JOIN LATERAL (SELECT id FROM users ORDER BY id LIMIT 1) fallback_user;
DROP TABLE integration_tokens_old;
ALTER TABLE ONLY integration_tokens
    ADD CONSTRAINT integration_tokens_pkey PRIMARY KEY (id);
ALTER TABLE ONLY integration_tokens
    ADD CONSTRAINT integration_tokens_token_hash_key UNIQUE (token_hash);

DELETE FROM media_files
WHERE title_id IS NULL
   OR NOT EXISTS (SELECT 1 FROM titles WHERE titles.id = media_files.title_id);
UPDATE media_files
SET
    file_path = COALESCE(NULLIF(file_path, ''), id),
    size_bytes = COALESCE(size_bytes, 0),
    scan_status = COALESCE(NULLIF(scan_status, ''), 'pending'),
    created_at = COALESCE(created_at, NOW());
ALTER TABLE media_files
    ALTER COLUMN title_id SET NOT NULL,
    ALTER COLUMN file_path SET NOT NULL,
    ALTER COLUMN size_bytes SET NOT NULL,
    ALTER COLUMN scan_status SET DEFAULT 'pending',
    ALTER COLUMN scan_status SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    DROP COLUMN episode_id;

ALTER TABLE file_episode_map
    ADD COLUMN is_filler boolean DEFAULT false;
UPDATE file_episode_map
SET is_filler = COALESCE(is_filler, false);
ALTER TABLE file_episode_map
    DROP COLUMN created_at;

ALTER TABLE library_scan_unmatched_items
    DROP COLUMN metadata_json;

ALTER TABLE push_subscriptions
    ADD COLUMN last_used_at timestamp with time zone;
UPDATE push_subscriptions
SET
    endpoint = COALESCE(NULLIF(endpoint, ''), id),
    p256dh = COALESCE(p256dh, ''),
    auth = COALESCE(auth, ''),
    created_at = COALESCE(created_at, NOW()),
    last_used_at = COALESCE(last_used_at, updated_at);
ALTER TABLE push_subscriptions
    ALTER COLUMN endpoint SET NOT NULL,
    ALTER COLUMN p256dh SET NOT NULL,
    ALTER COLUMN auth SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    DROP COLUMN updated_at;

DROP TABLE IF EXISTS mediarr_schema_migrations;

ALTER TABLE releases
    ADD COLUMN episode_id text,
    ADD COLUMN external_id text,
    ADD COLUMN title text,
    ADD COLUMN download_hint text,
    ADD COLUMN link text,
    ADD COLUMN size_bytes bigint,
    ADD COLUMN published_at timestamp with time zone,
    ADD COLUMN language_raw text,
    ADD COLUMN quality_label text,
    ADD COLUMN raw_payload_json jsonb,
    ADD COLUMN parsed_payload_json jsonb,
    ADD COLUMN last_seen_at timestamp with time zone;
UPDATE releases
SET
    title = COALESCE(NULLIF(title, ''), NULLIF(release_title, ''), external_id, id),
    link = COALESCE(link, release_url),
    size_bytes = COALESCE(size_bytes, release_size_bytes),
    raw_payload_json = COALESCE(raw_payload_json, payload_json),
    parsed_payload_json = COALESCE(parsed_payload_json, payload_json),
    last_seen_at = COALESCE(last_seen_at, updated_at, created_at, NOW()),
    created_at = COALESCE(created_at, NOW());
ALTER TABLE releases
    ALTER COLUMN title SET NOT NULL,
    ALTER COLUMN last_seen_at SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    DROP COLUMN release_title,
    DROP COLUMN release_url,
    DROP COLUMN release_size_bytes,
    DROP COLUMN release_score,
    DROP COLUMN payload_json,
    DROP COLUMN updated_at;

ALTER TABLE release_download_attempts
    ADD COLUMN created_at timestamp with time zone,
    ADD COLUMN updated_at timestamp with time zone;
UPDATE release_download_attempts
SET
    created_at = COALESCE(created_at, attempted_at, NOW()),
    updated_at = COALESCE(updated_at, attempted_at, NOW());
ALTER TABLE release_download_attempts
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE rule_set_history
    ADD COLUMN action text,
    ADD COLUMN rego_source text,
    ADD COLUMN actor_id text;
UPDATE rule_set_history
SET
    action = COALESCE(NULLIF(action, ''), event_json ->> 'action', 'updated'),
    rego_source = COALESCE(rego_source, event_json ->> 'rego_source'),
    actor_id = COALESCE(actor_id, event_json ->> 'actor_id'),
    created_at = COALESCE(created_at, NOW());
ALTER TABLE rule_set_history
    ALTER COLUMN action SET NOT NULL,
    ALTER COLUMN created_at SET DEFAULT NOW(),
    ALTER COLUMN created_at SET NOT NULL,
    DROP COLUMN event_json;

ALTER TABLE scheduler_jobs
    ADD COLUMN schedule_cron text,
    ADD COLUMN last_run_at timestamp with time zone,
    ADD COLUMN last_result text;
UPDATE scheduler_jobs
SET
    job_name = COALESCE(NULLIF(job_name, ''), id),
    payload_json = COALESCE(payload_json, '{}'::jsonb),
    status = COALESCE(NULLIF(status, ''), 'enabled'),
    created_at = COALESCE(created_at, NOW()),
    updated_at = COALESCE(updated_at, NOW());
ALTER TABLE scheduler_jobs
    ALTER COLUMN job_name SET NOT NULL,
    ALTER COLUMN payload_json SET NOT NULL,
    ALTER COLUMN status SET DEFAULT 'enabled',
    ALTER COLUMN status SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET NOT NULL;

DELETE FROM quality_profile_quality_tiers
USING quality_profiles_json snapshots
WHERE EXISTS (
    SELECT 1
    FROM quality_profiles profiles
    WHERE profiles.id = quality_profile_quality_tiers.profile_id
      AND profiles.scope = snapshots.scope
      AND COALESCE(profiles.scope_id, '') = COALESCE(snapshots.scope_id, '')
);
DELETE FROM quality_profile_source_allowlist
USING quality_profiles_json snapshots
WHERE EXISTS (
    SELECT 1
    FROM quality_profiles profiles
    WHERE profiles.id = quality_profile_source_allowlist.profile_id
      AND profiles.scope = snapshots.scope
      AND COALESCE(profiles.scope_id, '') = COALESCE(snapshots.scope_id, '')
);
DELETE FROM quality_profile_source_blocklist
USING quality_profiles_json snapshots
WHERE EXISTS (
    SELECT 1
    FROM quality_profiles profiles
    WHERE profiles.id = quality_profile_source_blocklist.profile_id
      AND profiles.scope = snapshots.scope
      AND COALESCE(profiles.scope_id, '') = COALESCE(snapshots.scope_id, '')
);
DELETE FROM quality_profile_video_codec_allowlist
USING quality_profiles_json snapshots
WHERE EXISTS (
    SELECT 1
    FROM quality_profiles profiles
    WHERE profiles.id = quality_profile_video_codec_allowlist.profile_id
      AND profiles.scope = snapshots.scope
      AND COALESCE(profiles.scope_id, '') = COALESCE(snapshots.scope_id, '')
);
DELETE FROM quality_profile_video_codec_blocklist
USING quality_profiles_json snapshots
WHERE EXISTS (
    SELECT 1
    FROM quality_profiles profiles
    WHERE profiles.id = quality_profile_video_codec_blocklist.profile_id
      AND profiles.scope = snapshots.scope
      AND COALESCE(profiles.scope_id, '') = COALESCE(snapshots.scope_id, '')
);
DELETE FROM quality_profile_audio_codec_allowlist
USING quality_profiles_json snapshots
WHERE EXISTS (
    SELECT 1
    FROM quality_profiles profiles
    WHERE profiles.id = quality_profile_audio_codec_allowlist.profile_id
      AND profiles.scope = snapshots.scope
      AND COALESCE(profiles.scope_id, '') = COALESCE(snapshots.scope_id, '')
);
DELETE FROM quality_profile_audio_codec_blocklist
USING quality_profiles_json snapshots
WHERE EXISTS (
    SELECT 1
    FROM quality_profiles profiles
    WHERE profiles.id = quality_profile_audio_codec_blocklist.profile_id
      AND profiles.scope = snapshots.scope
      AND COALESCE(profiles.scope_id, '') = COALESCE(snapshots.scope_id, '')
);
DELETE FROM quality_profiles profiles
USING quality_profiles_json snapshots
WHERE profiles.scope = snapshots.scope
  AND COALESCE(profiles.scope_id, '') = COALESCE(snapshots.scope_id, '');

WITH expanded AS (
    SELECT
        snapshots.scope,
        snapshots.scope_id,
        profile,
        COALESCE(profile -> 'criteria', '{}'::jsonb) AS criteria
    FROM quality_profiles_json snapshots
    CROSS JOIN LATERAL jsonb_array_elements(snapshots.profiles_json) AS profile
)
INSERT INTO quality_profiles (
    id,
    name,
    scope,
    scope_id,
    archival_quality,
    allow_unknown_quality,
    atmos_preferred,
    dolby_vision_allowed,
    detected_hdr_allowed,
    prefer_remux,
    allow_bd_disk,
    allow_upgrades,
    prefer_dual_audio,
    required_audio_languages,
    scoring_config,
    created_at
)
SELECT
    profile ->> 'id',
    COALESCE(NULLIF(profile ->> 'name', ''), profile ->> 'id'),
    scope,
    scope_id,
    criteria ->> 'archival_quality',
    COALESCE((criteria ->> 'allow_unknown_quality')::boolean, false),
    COALESCE((criteria ->> 'atmos_preferred')::boolean, false),
    COALESCE((criteria ->> 'dolby_vision_allowed')::boolean, false),
    COALESCE((criteria ->> 'detected_hdr_allowed')::boolean, true),
    COALESCE((criteria ->> 'prefer_remux')::boolean, false),
    COALESCE((criteria ->> 'allow_bd_disk')::boolean, false),
    COALESCE((criteria ->> 'allow_upgrades')::boolean, true),
    COALESCE((criteria ->> 'prefer_dual_audio')::boolean, false),
    CASE
        WHEN jsonb_typeof(criteria -> 'required_audio_languages') = 'array'
        THEN criteria -> 'required_audio_languages'
        ELSE '[]'::jsonb
    END,
    jsonb_build_object(
        'scoring_persona', COALESCE(criteria -> 'scoring_persona', '"balanced"'::jsonb),
        'scoring_overrides', COALESCE(criteria -> 'scoring_overrides', '{}'::jsonb),
        'cutoff_tier', criteria -> 'cutoff_tier',
        'min_score_to_grab', criteria -> 'min_score_to_grab',
        'facet_persona_overrides', COALESCE(criteria -> 'facet_persona_overrides', '{}'::jsonb)
    ),
    NOW()
FROM expanded
WHERE NULLIF(profile ->> 'id', '') IS NOT NULL
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    scope = EXCLUDED.scope,
    scope_id = EXCLUDED.scope_id,
    archival_quality = EXCLUDED.archival_quality,
    allow_unknown_quality = EXCLUDED.allow_unknown_quality,
    atmos_preferred = EXCLUDED.atmos_preferred,
    dolby_vision_allowed = EXCLUDED.dolby_vision_allowed,
    detected_hdr_allowed = EXCLUDED.detected_hdr_allowed,
    prefer_remux = EXCLUDED.prefer_remux,
    allow_bd_disk = EXCLUDED.allow_bd_disk,
    allow_upgrades = EXCLUDED.allow_upgrades,
    prefer_dual_audio = EXCLUDED.prefer_dual_audio,
    required_audio_languages = EXCLUDED.required_audio_languages,
    scoring_config = EXCLUDED.scoring_config;

WITH expanded AS (
    SELECT profile ->> 'id' AS profile_id, COALESCE(profile -> 'criteria', '{}'::jsonb) AS criteria
    FROM quality_profiles_json snapshots
    CROSS JOIN LATERAL jsonb_array_elements(snapshots.profiles_json) AS profile
)
INSERT INTO quality_profile_quality_tiers (profile_id, quality_tier, sort_order)
SELECT profile_id, value, ordinal - 1
FROM expanded
CROSS JOIN LATERAL jsonb_array_elements_text(
    CASE WHEN jsonb_typeof(criteria -> 'quality_tiers') = 'array'
         THEN criteria -> 'quality_tiers'
         ELSE '[]'::jsonb
    END
) WITH ORDINALITY AS tiers(value, ordinal)
ON CONFLICT DO NOTHING;

WITH expanded AS (
    SELECT profile ->> 'id' AS profile_id, COALESCE(profile -> 'criteria', '{}'::jsonb) AS criteria
    FROM quality_profiles_json snapshots
    CROSS JOIN LATERAL jsonb_array_elements(snapshots.profiles_json) AS profile
)
INSERT INTO quality_profile_source_allowlist (profile_id, source)
SELECT profile_id, value
FROM expanded
CROSS JOIN LATERAL jsonb_array_elements_text(
    CASE WHEN jsonb_typeof(criteria -> 'source_allowlist') = 'array'
         THEN criteria -> 'source_allowlist'
         ELSE '[]'::jsonb
    END
) AS values(value)
ON CONFLICT DO NOTHING;

WITH expanded AS (
    SELECT profile ->> 'id' AS profile_id, COALESCE(profile -> 'criteria', '{}'::jsonb) AS criteria
    FROM quality_profiles_json snapshots
    CROSS JOIN LATERAL jsonb_array_elements(snapshots.profiles_json) AS profile
)
INSERT INTO quality_profile_source_blocklist (profile_id, source)
SELECT profile_id, value
FROM expanded
CROSS JOIN LATERAL jsonb_array_elements_text(
    CASE WHEN jsonb_typeof(criteria -> 'source_blocklist') = 'array'
         THEN criteria -> 'source_blocklist'
         ELSE '[]'::jsonb
    END
) AS values(value)
ON CONFLICT DO NOTHING;

WITH expanded AS (
    SELECT profile ->> 'id' AS profile_id, COALESCE(profile -> 'criteria', '{}'::jsonb) AS criteria
    FROM quality_profiles_json snapshots
    CROSS JOIN LATERAL jsonb_array_elements(snapshots.profiles_json) AS profile
)
INSERT INTO quality_profile_video_codec_allowlist (profile_id, codec)
SELECT profile_id, value
FROM expanded
CROSS JOIN LATERAL jsonb_array_elements_text(
    CASE WHEN jsonb_typeof(criteria -> 'video_codec_allowlist') = 'array'
         THEN criteria -> 'video_codec_allowlist'
         ELSE '[]'::jsonb
    END
) AS values(value)
ON CONFLICT DO NOTHING;

WITH expanded AS (
    SELECT profile ->> 'id' AS profile_id, COALESCE(profile -> 'criteria', '{}'::jsonb) AS criteria
    FROM quality_profiles_json snapshots
    CROSS JOIN LATERAL jsonb_array_elements(snapshots.profiles_json) AS profile
)
INSERT INTO quality_profile_video_codec_blocklist (profile_id, codec)
SELECT profile_id, value
FROM expanded
CROSS JOIN LATERAL jsonb_array_elements_text(
    CASE WHEN jsonb_typeof(criteria -> 'video_codec_blocklist') = 'array'
         THEN criteria -> 'video_codec_blocklist'
         ELSE '[]'::jsonb
    END
) AS values(value)
ON CONFLICT DO NOTHING;

WITH expanded AS (
    SELECT profile ->> 'id' AS profile_id, COALESCE(profile -> 'criteria', '{}'::jsonb) AS criteria
    FROM quality_profiles_json snapshots
    CROSS JOIN LATERAL jsonb_array_elements(snapshots.profiles_json) AS profile
)
INSERT INTO quality_profile_audio_codec_allowlist (profile_id, codec)
SELECT profile_id, value
FROM expanded
CROSS JOIN LATERAL jsonb_array_elements_text(
    CASE WHEN jsonb_typeof(criteria -> 'audio_codec_allowlist') = 'array'
         THEN criteria -> 'audio_codec_allowlist'
         ELSE '[]'::jsonb
    END
) AS values(value)
ON CONFLICT DO NOTHING;

WITH expanded AS (
    SELECT profile ->> 'id' AS profile_id, COALESCE(profile -> 'criteria', '{}'::jsonb) AS criteria
    FROM quality_profiles_json snapshots
    CROSS JOIN LATERAL jsonb_array_elements(snapshots.profiles_json) AS profile
)
INSERT INTO quality_profile_audio_codec_blocklist (profile_id, codec)
SELECT profile_id, value
FROM expanded
CROSS JOIN LATERAL jsonb_array_elements_text(
    CASE WHEN jsonb_typeof(criteria -> 'audio_codec_blocklist') = 'array'
         THEN criteria -> 'audio_codec_blocklist'
         ELSE '[]'::jsonb
    END
) AS values(value)
ON CONFLICT DO NOTHING;

UPDATE quality_profiles
SET
    name = COALESCE(NULLIF(name, ''), id),
    scope = COALESCE(NULLIF(scope, ''), 'system'),
    required_audio_languages = COALESCE(required_audio_languages, '[]'::jsonb),
    scoring_config = COALESCE(scoring_config, '{}'::jsonb),
    created_at = COALESCE(created_at, NOW());
ALTER TABLE quality_profiles
    ALTER COLUMN name SET NOT NULL,
    ALTER COLUMN scope SET NOT NULL,
    ALTER COLUMN allow_unknown_quality SET DEFAULT false,
    ALTER COLUMN allow_unknown_quality SET NOT NULL,
    ALTER COLUMN atmos_preferred SET DEFAULT false,
    ALTER COLUMN atmos_preferred SET NOT NULL,
    ALTER COLUMN dolby_vision_allowed SET DEFAULT false,
    ALTER COLUMN dolby_vision_allowed SET NOT NULL,
    ALTER COLUMN detected_hdr_allowed SET DEFAULT true,
    ALTER COLUMN detected_hdr_allowed SET NOT NULL,
    ALTER COLUMN prefer_remux SET DEFAULT false,
    ALTER COLUMN prefer_remux SET NOT NULL,
    ALTER COLUMN allow_bd_disk SET DEFAULT false,
    ALTER COLUMN allow_bd_disk SET NOT NULL,
    ALTER COLUMN allow_upgrades SET DEFAULT true,
    ALTER COLUMN allow_upgrades SET NOT NULL,
    ALTER COLUMN prefer_dual_audio SET DEFAULT false,
    ALTER COLUMN prefer_dual_audio SET NOT NULL,
    ALTER COLUMN required_audio_languages SET DEFAULT '[]'::jsonb,
    ALTER COLUMN required_audio_languages SET NOT NULL,
    ALTER COLUMN scoring_config SET DEFAULT '{}'::jsonb,
    ALTER COLUMN scoring_config SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL;

ALTER TABLE settings_definitions
    DROP CONSTRAINT settings_definitions_scope_key_name_key;
DELETE FROM settings_definitions a
USING settings_definitions b
WHERE a.ctid < b.ctid
  AND a.category = b.category
  AND a.scope = b.scope
  AND a.key_name = b.key_name;
ALTER TABLE ONLY settings_definitions
    ADD CONSTRAINT settings_definitions_category_scope_key_name_key UNIQUE (category, scope, key_name);

UPDATE settings_values
SET
    value_json = COALESCE(value_json, '{}'::jsonb),
    source = COALESCE(NULLIF(source, ''), 'system'),
    created_at = COALESCE(created_at, NOW()),
    updated_at = COALESCE(updated_at, NOW());
DELETE FROM settings_values a
USING settings_values b
WHERE a.ctid < b.ctid
  AND a.setting_definition_id = b.setting_definition_id
  AND a.scope = b.scope
  AND COALESCE(a.scope_id, '') = COALESCE(b.scope_id, '');
ALTER TABLE settings_values
    ALTER COLUMN value_json SET NOT NULL,
    ALTER COLUMN source SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE subtitle_blocklist DROP CONSTRAINT subtitle_blocklist_pkey;
UPDATE subtitle_blocklist
SET
    id = COALESCE(NULLIF(id, ''), md5(media_file_id || ':' || provider || ':' || provider_file_id)),
    language = COALESCE(NULLIF(language, ''), 'und'),
    created_at = COALESCE(created_at, NOW());
DELETE FROM subtitle_blocklist a
USING subtitle_blocklist b
WHERE a.ctid < b.ctid
  AND a.id = b.id;
ALTER TABLE subtitle_blocklist
    ALTER COLUMN id SET NOT NULL,
    ALTER COLUMN language SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL;
ALTER TABLE ONLY subtitle_blocklist
    ADD CONSTRAINT subtitle_blocklist_pkey PRIMARY KEY (id);
ALTER TABLE ONLY subtitle_blocklist
    ADD CONSTRAINT subtitle_blocklist_media_file_provider_provider_file_id_key
    UNIQUE (media_file_id, provider, provider_file_id);

DROP INDEX IF EXISTS idx_title_external_ids_facet_lookup;
DROP INDEX IF EXISTS idx_title_external_ids_lookup;
ALTER TABLE title_external_ids
    DROP COLUMN provenance,
    DROP COLUMN source_scope;

DELETE FROM title_image_variants
WHERE title_image_id IS NULL
   OR NOT EXISTS (SELECT 1 FROM title_images WHERE title_images.id = title_image_variants.title_image_id);
UPDATE title_image_variants
SET
    variant_key = COALESCE(NULLIF(variant_key, ''), 'original'),
    path = COALESCE(path, ''),
    format = COALESCE(NULLIF(format, ''), 'webp'),
    width = COALESCE(width, 0),
    height = COALESCE(height, 0),
    bytes = COALESCE(bytes, ''::bytea),
    sha256 = COALESCE(NULLIF(sha256, ''), md5(COALESCE(bytes, ''::bytea)::text)),
    created_at = COALESCE(created_at, NOW()),
    updated_at = COALESCE(updated_at, NOW());
ALTER TABLE title_image_variants
    ALTER COLUMN title_image_id SET NOT NULL,
    ALTER COLUMN variant_key SET NOT NULL,
    ALTER COLUMN format SET NOT NULL,
    ALTER COLUMN width SET NOT NULL,
    ALTER COLUMN height SET NOT NULL,
    ALTER COLUMN bytes SET NOT NULL,
    ALTER COLUMN sha256 SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET NOT NULL;

DELETE FROM title_images
WHERE title_id IS NULL
   OR NOT EXISTS (SELECT 1 FROM titles WHERE titles.id = title_images.title_id);
UPDATE title_images
SET
    provider = COALESCE(NULLIF(provider, ''), 'unknown'),
    kind = COALESCE(NULLIF(kind, ''), 'poster'),
    source_url = COALESCE(NULLIF(source_url, ''), ''),
    source_format = COALESCE(NULLIF(source_format, ''), 'unknown'),
    storage_mode = COALESCE(NULLIF(storage_mode, ''), 'database'),
    master_format = COALESCE(NULLIF(master_format, ''), COALESCE(NULLIF(source_format, ''), 'unknown')),
    master_sha256 = COALESCE(NULLIF(master_sha256, ''), md5(COALESCE(bytes, ''::bytea)::text)),
    master_width = COALESCE(master_width, 0),
    master_height = COALESCE(master_height, 0),
    bytes = COALESCE(bytes, ''::bytea),
    created_at = COALESCE(created_at, NOW()),
    updated_at = COALESCE(updated_at, NOW());
ALTER TABLE title_images
    ALTER COLUMN title_id SET NOT NULL,
    ALTER COLUMN provider SET NOT NULL,
    ALTER COLUMN kind SET NOT NULL,
    ALTER COLUMN source_url SET NOT NULL,
    ALTER COLUMN source_format SET NOT NULL,
    ALTER COLUMN storage_mode SET NOT NULL,
    ALTER COLUMN master_format SET NOT NULL,
    ALTER COLUMN master_sha256 SET NOT NULL,
    ALTER COLUMN master_width SET NOT NULL,
    ALTER COLUMN master_height SET NOT NULL,
    ALTER COLUMN bytes SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE title_search_terms DROP COLUMN updated_at;

ALTER TABLE users
    ADD COLUMN display_name text,
    ADD COLUMN status text DEFAULT 'active'::text NOT NULL,
    ADD COLUMN passkey_public_key text,
    ADD COLUMN locale text,
    ADD COLUMN created_at timestamp with time zone DEFAULT NOW() NOT NULL,
    ADD COLUMN updated_at timestamp with time zone DEFAULT NOW() NOT NULL,
    ADD COLUMN last_login_at timestamp with time zone;

DELETE FROM wanted_items
WHERE title_id IS NULL
   OR NOT EXISTS (SELECT 1 FROM titles WHERE titles.id = wanted_items.title_id);
UPDATE wanted_items
SET
    media_type = COALESCE(NULLIF(media_type, ''), 'movie'),
    search_phase = COALESCE(NULLIF(search_phase, ''), 'primary'),
    search_count = COALESCE(search_count, 0),
    status = COALESCE(NULLIF(status, ''), 'wanted'),
    created_at = COALESCE(created_at, NOW()),
    updated_at = COALESCE(updated_at, NOW());
DELETE FROM wanted_items a
USING wanted_items b
WHERE a.ctid < b.ctid
  AND a.title_id = b.title_id
  AND a.episode_id IS NOT DISTINCT FROM b.episode_id;
DROP INDEX IF EXISTS idx_wanted_items_episode_unique;
ALTER TABLE wanted_items
    ALTER COLUMN title_id SET NOT NULL,
    ALTER COLUMN media_type SET NOT NULL,
    ALTER COLUMN search_phase SET DEFAULT 'primary',
    ALTER COLUMN search_phase SET NOT NULL,
    ALTER COLUMN search_count SET DEFAULT 0,
    ALTER COLUMN search_count SET NOT NULL,
    ALTER COLUMN status SET DEFAULT 'wanted',
    ALTER COLUMN status SET NOT NULL,
    ALTER COLUMN created_at SET NOT NULL,
    ALTER COLUMN updated_at SET NOT NULL;
ALTER TABLE ONLY wanted_items
    ADD CONSTRAINT wanted_items_title_episode_key UNIQUE (title_id, episode_id);

UPDATE download_queue_commands
SET client_id = NULL
WHERE client_id = '';
ALTER TABLE download_queue_commands
    ALTER COLUMN client_id DROP DEFAULT,
    ALTER COLUMN client_id DROP NOT NULL;

DROP INDEX IF EXISTS idx_download_queue_commands_active_unique;
DROP INDEX IF EXISTS idx_library_scan_unmatched_items_facet_path;
DROP INDEX IF EXISTS idx_quality_profiles_json_scope;
DROP INDEX IF EXISTS idx_settings_values_definition_scope;
DROP INDEX IF EXISTS idx_titles_facet_library_name_id;
DROP INDEX IF EXISTS idx_titles_facet_slug_library_id;
DROP INDEX IF EXISTS idx_titles_metadata_hydration_due;
DROP INDEX IF EXISTS idx_title_search_terms_facet_normalized;
DROP INDEX IF EXISTS idx_title_search_terms_title;
DROP INDEX IF EXISTS idx_title_search_terms_unique;

CREATE INDEX idx_collection_external_ids_title_provenance
    ON collection_external_ids (title_id, provenance);
CREATE INDEX idx_download_jobs_client
    ON download_jobs (download_client_id, status);
CREATE INDEX idx_download_jobs_workflow
    ON download_jobs (workflow_operation_id);
CREATE UNIQUE INDEX idx_download_queue_commands_active_unique
    ON download_queue_commands (action, COALESCE(client_id, ''), client_type, download_client_item_id, is_history)
    WHERE status IN ('queued', 'running');
CREATE INDEX idx_episode_external_ids_title_provenance
    ON episode_external_ids (title_id, provenance);
CREATE INDEX idx_event_outboxes_channel
    ON event_outboxes (channel_key);
CREATE INDEX idx_history_title_time
    ON history_events (title_id, occurred_at DESC);
CREATE INDEX idx_history_type_time
    ON history_events (event_type, occurred_at DESC);
CREATE INDEX idx_integration_tokens_user
    ON integration_tokens (user_id);
CREATE INDEX idx_pp_script_runs_title_id
    ON post_processing_script_runs (title_id, started_at DESC);
CREATE UNIQUE INDEX idx_setting_values_scope_name
    ON settings_values (setting_definition_id, scope, COALESCE(scope_id, ''));
CREATE INDEX idx_settings_values_definition
    ON settings_values (setting_definition_id);
CREATE INDEX idx_title_image_variants_image_variant
    ON title_image_variants (title_image_id, variant_key);
CREATE INDEX idx_title_images_title_kind
    ON title_images (title_id, kind);
CREATE INDEX idx_title_search_terms_facet_normalized_term
    ON title_search_terms (facet, normalized_term);
CREATE INDEX idx_title_search_terms_title_id
    ON title_search_terms (title_id);
CREATE UNIQUE INDEX idx_title_search_terms_title_kind_normalized
    ON title_search_terms (title_id, term_kind, normalized_term);
CREATE INDEX idx_titles_metadata_hydration_due
    ON titles (metadata_hydration_next_attempt_at, metadata_fetched_at);

DROP TABLE IF EXISTS import_artifacts;
DROP TABLE IF EXISTS job_runs;
DROP TABLE IF EXISTS quality_profiles_json;

UPDATE releases SET title_id = NULL
WHERE title_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM titles WHERE titles.id = releases.title_id);
UPDATE releases SET collection_id = NULL
WHERE collection_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM collections WHERE collections.id = releases.collection_id);
UPDATE releases SET episode_id = NULL
WHERE episode_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM episodes WHERE episodes.id = releases.episode_id);
UPDATE releases SET indexer_id = NULL
WHERE indexer_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM indexers WHERE indexers.id = releases.indexer_id);
DELETE FROM blocklist
WHERE title_id IS NULL
   OR NOT EXISTS (SELECT 1 FROM titles WHERE titles.id = blocklist.title_id);
DELETE FROM external_subtitle_probe_cache
WHERE media_file_id IS NULL
   OR NOT EXISTS (SELECT 1 FROM media_files WHERE media_files.id = external_subtitle_probe_cache.media_file_id);
UPDATE download_import_artifacts SET import_id = NULL
WHERE import_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM imports WHERE imports.id = download_import_artifacts.import_id);
UPDATE download_import_artifacts SET title_id = NULL
WHERE title_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM titles WHERE titles.id = download_import_artifacts.title_id);
UPDATE download_import_artifacts SET episode_id = NULL
WHERE episode_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM episodes WHERE episodes.id = download_import_artifacts.episode_id);
UPDATE download_import_artifacts SET imported_media_file_id = NULL
WHERE imported_media_file_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM media_files WHERE media_files.id = download_import_artifacts.imported_media_file_id);
DELETE FROM download_submission_episode_links links
WHERE NOT EXISTS (
    SELECT 1
    FROM download_submissions submissions
    WHERE submissions.download_client_id = links.download_client_id
      AND submissions.download_client_type = links.download_client_type
      AND submissions.download_client_item_id = links.download_client_item_id
);
DELETE FROM file_episode_map
WHERE NOT EXISTS (SELECT 1 FROM media_files WHERE media_files.id = file_episode_map.file_id)
   OR NOT EXISTS (SELECT 1 FROM episodes WHERE episodes.id = file_episode_map.episode_id);
UPDATE history_events SET actor_user_id = NULL
WHERE actor_user_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM users WHERE users.id = history_events.actor_user_id);
UPDATE history_events SET title_id = NULL
WHERE title_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM titles WHERE titles.id = history_events.title_id);
DELETE FROM library_probe_signatures
WHERE title_id IS NULL
   OR NOT EXISTS (SELECT 1 FROM titles WHERE titles.id = library_probe_signatures.title_id);
DELETE FROM post_processing_script_runs
WHERE script_id IS NULL
   OR NOT EXISTS (SELECT 1 FROM post_processing_scripts WHERE post_processing_scripts.id = post_processing_script_runs.script_id);
DELETE FROM quality_profile_audio_codec_allowlist
WHERE NOT EXISTS (SELECT 1 FROM quality_profiles WHERE quality_profiles.id = quality_profile_audio_codec_allowlist.profile_id);
DELETE FROM quality_profile_audio_codec_blocklist
WHERE NOT EXISTS (SELECT 1 FROM quality_profiles WHERE quality_profiles.id = quality_profile_audio_codec_blocklist.profile_id);
DELETE FROM quality_profile_source_allowlist
WHERE NOT EXISTS (SELECT 1 FROM quality_profiles WHERE quality_profiles.id = quality_profile_source_allowlist.profile_id);
DELETE FROM quality_profile_source_blocklist
WHERE NOT EXISTS (SELECT 1 FROM quality_profiles WHERE quality_profiles.id = quality_profile_source_blocklist.profile_id);
DELETE FROM quality_profile_video_codec_allowlist
WHERE NOT EXISTS (SELECT 1 FROM quality_profiles WHERE quality_profiles.id = quality_profile_video_codec_allowlist.profile_id);
DELETE FROM quality_profile_video_codec_blocklist
WHERE NOT EXISTS (SELECT 1 FROM quality_profiles WHERE quality_profiles.id = quality_profile_video_codec_blocklist.profile_id);
UPDATE quarantine_items SET media_file_id = NULL
WHERE media_file_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM media_files WHERE media_files.id = quarantine_items.media_file_id);
UPDATE quarantine_items SET quarantined_by = NULL
WHERE quarantined_by IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM users WHERE users.id = quarantine_items.quarantined_by);
UPDATE quarantine_items SET release_id = NULL
WHERE release_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM releases WHERE releases.id = quarantine_items.release_id);
UPDATE release_download_attempts SET title_id = NULL
WHERE title_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM titles WHERE titles.id = release_download_attempts.title_id);
DELETE FROM release_decisions
WHERE wanted_item_id IS NULL
   OR NOT EXISTS (SELECT 1 FROM wanted_items WHERE wanted_items.id = release_decisions.wanted_item_id);
DELETE FROM subtitle_downloads
WHERE media_file_id IS NULL
   OR title_id IS NULL
   OR NOT EXISTS (SELECT 1 FROM media_files WHERE media_files.id = subtitle_downloads.media_file_id)
   OR NOT EXISTS (SELECT 1 FROM titles WHERE titles.id = subtitle_downloads.title_id);
DELETE FROM title_aliases
WHERE title_id IS NULL
   OR NOT EXISTS (SELECT 1 FROM titles WHERE titles.id = title_aliases.title_id);
UPDATE upgrades SET workflow_operation_id = NULL
WHERE workflow_operation_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM workflow_operations WHERE workflow_operations.id = upgrades.workflow_operation_id);
UPDATE upgrades SET actor_user_id = NULL
WHERE actor_user_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM users WHERE users.id = upgrades.actor_user_id);
DELETE FROM push_subscriptions a
USING push_subscriptions b
WHERE a.ctid < b.ctid
  AND a.endpoint = b.endpoint;
DELETE FROM wanted_items
WHERE title_id IS NULL
   OR NOT EXISTS (SELECT 1 FROM titles WHERE titles.id = wanted_items.title_id)
   OR (episode_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM episodes WHERE episodes.id = wanted_items.episode_id));
UPDATE wanted_items SET collection_id = NULL
WHERE collection_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM collections WHERE collections.id = wanted_items.collection_id);
UPDATE workflow_operations SET actor_user_id = NULL
WHERE actor_user_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM users WHERE users.id = workflow_operations.actor_user_id);
UPDATE workflow_operations SET title_id = NULL
WHERE title_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM titles WHERE titles.id = workflow_operations.title_id);
UPDATE workflow_operations SET collection_id = NULL
WHERE collection_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM collections WHERE collections.id = workflow_operations.collection_id);
UPDATE workflow_operations SET episode_id = NULL
WHERE episode_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM episodes WHERE episodes.id = workflow_operations.episode_id);
UPDATE workflow_operations SET release_id = NULL
WHERE release_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM releases WHERE releases.id = workflow_operations.release_id);
UPDATE workflow_operations SET media_file_id = NULL
WHERE media_file_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM media_files WHERE media_files.id = workflow_operations.media_file_id);

ALTER TABLE ONLY blocklist
    ADD CONSTRAINT blocklist_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY collection_external_ids
    ADD CONSTRAINT collection_external_ids_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY collection_external_ids
    ADD CONSTRAINT collection_external_ids_collection_id_fkey FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE CASCADE;
ALTER TABLE ONLY external_subtitle_probe_cache
    ADD CONSTRAINT external_subtitle_probe_cache_media_file_id_fkey FOREIGN KEY (media_file_id) REFERENCES media_files(id) ON DELETE CASCADE;
ALTER TABLE ONLY download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_import_id_fkey FOREIGN KEY (import_id) REFERENCES imports(id) ON DELETE SET NULL;
ALTER TABLE ONLY download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL;
ALTER TABLE ONLY download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_episode_id_fkey FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL;
ALTER TABLE ONLY download_import_artifacts
    ADD CONSTRAINT download_import_artifacts_imported_media_file_id_fkey FOREIGN KEY (imported_media_file_id) REFERENCES media_files(id) ON DELETE SET NULL;
ALTER TABLE ONLY download_submission_episode_links
    ADD CONSTRAINT download_submission_episode_links_submission_fkey FOREIGN KEY (download_client_id, download_client_type, download_client_item_id) REFERENCES download_submissions(download_client_id, download_client_type, download_client_item_id) ON DELETE CASCADE;
ALTER TABLE ONLY episode_external_ids
    ADD CONSTRAINT episode_external_ids_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY episode_external_ids
    ADD CONSTRAINT episode_external_ids_episode_id_fkey FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE CASCADE;
ALTER TABLE ONLY file_episode_map
    ADD CONSTRAINT file_episode_map_file_id_fkey FOREIGN KEY (file_id) REFERENCES media_files(id) ON DELETE CASCADE;
ALTER TABLE ONLY file_episode_map
    ADD CONSTRAINT file_episode_map_episode_id_fkey FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE CASCADE;
ALTER TABLE ONLY download_jobs
    ADD CONSTRAINT download_jobs_workflow_operation_id_fkey FOREIGN KEY (workflow_operation_id) REFERENCES workflow_operations(id) ON DELETE CASCADE;
ALTER TABLE ONLY download_jobs
    ADD CONSTRAINT download_jobs_download_client_id_fkey FOREIGN KEY (download_client_id) REFERENCES download_clients(id) ON DELETE RESTRICT;
ALTER TABLE ONLY download_jobs
    ADD CONSTRAINT download_jobs_release_id_fkey FOREIGN KEY (release_id) REFERENCES releases(id) ON DELETE SET NULL;
ALTER TABLE ONLY event_outboxes
    ADD CONSTRAINT event_outboxes_history_event_id_fkey FOREIGN KEY (history_event_id) REFERENCES history_events(id) ON DELETE CASCADE;
ALTER TABLE ONLY history_events
    ADD CONSTRAINT history_events_actor_user_id_fkey FOREIGN KEY (actor_user_id) REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE ONLY history_events
    ADD CONSTRAINT history_events_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL;
ALTER TABLE ONLY integration_tokens
    ADD CONSTRAINT integration_tokens_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE ONLY library_probe_signatures
    ADD CONSTRAINT library_probe_signatures_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY media_files
    ADD CONSTRAINT media_files_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY post_processing_script_runs
    ADD CONSTRAINT post_processing_script_runs_script_id_fkey FOREIGN KEY (script_id) REFERENCES post_processing_scripts(id) ON DELETE CASCADE;
ALTER TABLE ONLY push_subscriptions
    ADD CONSTRAINT push_subscriptions_endpoint_key UNIQUE (endpoint);
ALTER TABLE ONLY quality_profile_audio_codec_allowlist
    ADD CONSTRAINT quality_profile_audio_codec_allowlist_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY quality_profile_audio_codec_blocklist
    ADD CONSTRAINT quality_profile_audio_codec_blocklist_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY quality_profile_source_allowlist
    ADD CONSTRAINT quality_profile_source_allowlist_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY quality_profile_source_blocklist
    ADD CONSTRAINT quality_profile_source_blocklist_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY quality_profile_video_codec_allowlist
    ADD CONSTRAINT quality_profile_video_codec_allowlist_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY quality_profile_video_codec_blocklist
    ADD CONSTRAINT quality_profile_video_codec_blocklist_profile_id_fkey FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE;
ALTER TABLE ONLY quarantine_items
    ADD CONSTRAINT quarantine_items_media_file_id_fkey FOREIGN KEY (media_file_id) REFERENCES media_files(id) ON DELETE SET NULL;
ALTER TABLE ONLY quarantine_items
    ADD CONSTRAINT quarantine_items_quarantined_by_fkey FOREIGN KEY (quarantined_by) REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE ONLY quarantine_items
    ADD CONSTRAINT quarantine_items_release_id_fkey FOREIGN KEY (release_id) REFERENCES releases(id) ON DELETE SET NULL;
ALTER TABLE ONLY release_download_attempts
    ADD CONSTRAINT release_download_attempts_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL;
ALTER TABLE ONLY release_decisions
    ADD CONSTRAINT release_decisions_wanted_item_id_fkey FOREIGN KEY (wanted_item_id) REFERENCES wanted_items(id) ON DELETE CASCADE;
ALTER TABLE ONLY releases
    ADD CONSTRAINT releases_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL;
ALTER TABLE ONLY releases
    ADD CONSTRAINT releases_collection_id_fkey FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE SET NULL;
ALTER TABLE ONLY releases
    ADD CONSTRAINT releases_episode_id_fkey FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL;
ALTER TABLE ONLY releases
    ADD CONSTRAINT releases_indexer_id_fkey FOREIGN KEY (indexer_id) REFERENCES indexers(id) ON DELETE SET NULL;
ALTER TABLE ONLY subtitle_downloads
    ADD CONSTRAINT subtitle_downloads_media_file_id_fkey FOREIGN KEY (media_file_id) REFERENCES media_files(id) ON DELETE CASCADE;
ALTER TABLE ONLY subtitle_downloads
    ADD CONSTRAINT subtitle_downloads_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_image_variants
    ADD CONSTRAINT title_image_variants_title_image_id_fkey FOREIGN KEY (title_image_id) REFERENCES title_images(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_images
    ADD CONSTRAINT title_images_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY title_aliases
    ADD CONSTRAINT title_aliases_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY upgrades
    ADD CONSTRAINT upgrades_workflow_operation_id_fkey FOREIGN KEY (workflow_operation_id) REFERENCES workflow_operations(id) ON DELETE SET NULL;
ALTER TABLE ONLY upgrades
    ADD CONSTRAINT upgrades_actor_user_id_fkey FOREIGN KEY (actor_user_id) REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE ONLY user_entitlements
    DROP CONSTRAINT IF EXISTS user_entitlements_granted_by_user_id_fkey;
ALTER TABLE ONLY wanted_items
    ADD CONSTRAINT wanted_items_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE;
ALTER TABLE ONLY wanted_items
    ADD CONSTRAINT wanted_items_episode_id_fkey FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE CASCADE;
ALTER TABLE ONLY wanted_items
    ADD CONSTRAINT wanted_items_collection_id_fkey FOREIGN KEY (collection_id) REFERENCES collections(id);
ALTER TABLE ONLY workflow_operations
    ADD CONSTRAINT workflow_operations_actor_user_id_fkey FOREIGN KEY (actor_user_id) REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE ONLY workflow_operations
    ADD CONSTRAINT workflow_operations_title_id_fkey FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE SET NULL;
ALTER TABLE ONLY workflow_operations
    ADD CONSTRAINT workflow_operations_collection_id_fkey FOREIGN KEY (collection_id) REFERENCES collections(id) ON DELETE SET NULL;
ALTER TABLE ONLY workflow_operations
    ADD CONSTRAINT workflow_operations_episode_id_fkey FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE SET NULL;
ALTER TABLE ONLY workflow_operations
    ADD CONSTRAINT workflow_operations_release_id_fkey FOREIGN KEY (release_id) REFERENCES releases(id) ON DELETE SET NULL;
ALTER TABLE ONLY workflow_operations
    ADD CONSTRAINT workflow_operations_media_file_id_fkey FOREIGN KEY (media_file_id) REFERENCES media_files(id) ON DELETE SET NULL;
