ALTER TABLE subtitle_provider_configs
ADD COLUMN enabled_facets TEXT NOT NULL DEFAULT '[]';

UPDATE subtitle_provider_configs
SET enabled_facets = '["movie","series"]'
WHERE provider_type = 'opensubtitles' AND enabled_facets = '[]';
