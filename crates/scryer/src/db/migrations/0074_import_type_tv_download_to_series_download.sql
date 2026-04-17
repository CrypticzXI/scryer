-- Historical episodic imports used the old "tv_download" value before the
-- facet model was canonicalized to "series". Rewrite legacy rows forward so
-- runtime parsing only needs to support the canonical import type.
UPDATE imports
SET import_type = 'series_download'
WHERE import_type = 'tv_download';
