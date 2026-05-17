ALTER TABLE rule_sets
    ALTER COLUMN applied_facets TYPE text USING COALESCE(applied_facets::text, '[]'),
    ALTER COLUMN applied_facets SET DEFAULT '[]';

ALTER TABLE post_processing_scripts
    ALTER COLUMN applied_facets TYPE text USING COALESCE(applied_facets::text, '[]'),
    ALTER COLUMN applied_facets SET DEFAULT '[]';

ALTER TABLE plugin_installations
    ALTER COLUMN descriptor_json TYPE text USING descriptor_json::text;

ALTER TABLE notification_channels
    ALTER COLUMN config_json TYPE text USING CASE
        WHEN jsonb_typeof(config_json) = 'string' THEN config_json #>> '{}'
        ELSE config_json::text
    END;

ALTER TABLE library_scan_unmatched_items
    ALTER COLUMN search_attempts_json TYPE text USING COALESCE(search_attempts_json::text, '[]'),
    ALTER COLUMN search_attempts_json SET DEFAULT '[]';

ALTER TABLE media_files
    ALTER COLUMN analysis_json TYPE text USING analysis_json::text;

ALTER TABLE collections
    ALTER COLUMN interstitial_genres_json TYPE text USING interstitial_genres_json::text,
    ALTER COLUMN special_movies_json TYPE text USING COALESCE(special_movies_json::text, '[]'),
    ALTER COLUMN special_movies_json SET DEFAULT '[]';
